//! Topology walks for selection tools (Blender's Alt+click loops, L, Ctrl+L,
//! Ctrl+Numpad±) and duplication.

use std::collections::{HashMap, HashSet};

use crate::{PolyMesh, edge_key, loop_edges};

/// Edge → faces, vertex → neighbours and vertex → faces, built once per query.
pub struct Adjacency {
    pub edge_faces: HashMap<[u32; 2], Vec<usize>>,
    pub neighbors: Vec<Vec<u32>>,
    pub vertex_faces: Vec<Vec<usize>>,
}

impl Adjacency {
    /// Faces using the edge `a`-`b` (empty for wire edges and non-edges).
    pub fn faces(&self, a: u32, b: u32) -> &[usize] {
        self.edge_faces
            .get(&edge_key(a, b))
            .map_or(&[], Vec::as_slice)
    }

    /// The next vertex of the edge loop entering `v` from `u`: across a
    /// 4-valent vertex to the edge sharing no face with `u`-`v`, along the
    /// border for border edges, along wire chains for wire edges.
    fn loop_next(&self, u: u32, v: u32) -> Option<u32> {
        let current = self.faces(u, v);
        let around = &self.neighbors[v as usize];
        let mut next = around.iter().copied().filter(|&w| w != u);
        let unique = |it: &mut dyn Iterator<Item = u32>| {
            let w = it.next()?;
            it.next().is_none().then_some(w)
        };
        match current.len() {
            2 if around.len() == 4 => {
                unique(&mut next.filter(|&w| self.faces(v, w).iter().all(|f| !current.contains(f))))
            }
            1 => unique(&mut next.filter(|&w| self.faces(v, w).len() == 1)),
            0 if around.len() == 2 => unique(&mut next),
            _ => None,
        }
    }
}

impl PolyMesh {
    pub fn adjacency(&self) -> Adjacency {
        let mut edge_faces: HashMap<[u32; 2], Vec<usize>> = HashMap::new();
        let mut vertex_faces = vec![Vec::new(); self.positions.len()];
        for (f, face) in self.faces.iter().enumerate() {
            for (a, b) in loop_edges(face) {
                edge_faces.entry(edge_key(a, b)).or_default().push(f);
                vertex_faces[a as usize].push(f);
            }
        }
        let mut neighbors = vec![Vec::new(); self.positions.len()];
        let wire = self.wire.iter().filter(|e| !edge_faces.contains_key(*e));
        for &[a, b] in edge_faces.keys().chain(wire) {
            neighbors[a as usize].push(b);
            neighbors[b as usize].push(a);
        }
        Adjacency {
            edge_faces,
            neighbors,
            vertex_faces,
        }
    }

    /// Blender's edge loop (Alt+click) through the edge `a`-`b`, walked in
    /// both directions until a pole, a corner or the start. Unordered.
    pub fn edge_loop(&self, a: u32, b: u32) -> Vec<[u32; 2]> {
        if a == b || !self.has_edge(a, b) {
            return Vec::new();
        }
        let adj = self.adjacency();
        let mut out = vec![edge_key(a, b)];
        let mut seen: HashSet<[u32; 2]> = out.iter().copied().collect();
        for (mut u, mut v) in [(a, b), (b, a)] {
            while let Some(w) = adj.loop_next(u, v) {
                if !seen.insert(edge_key(v, w)) {
                    break;
                }
                out.push(edge_key(v, w));
                (u, v) = (v, w);
            }
        }
        out
    }

    /// Faces and edges of the quad ring crossing `a`-`b` (the faces a loop
    /// cut would split): Blender's face loop and edge ring.
    pub fn ring(&self, a: u32, b: u32) -> (Vec<usize>, Vec<[u32; 2]>) {
        let start = edge_key(a, b);
        let adj = self.adjacency();
        let (mut faces, mut edges) = (Vec::new(), vec![start]);
        let mut seen = HashSet::new();
        for &first in adj.faces(a, b).iter().take(2) {
            let (mut f, mut entry) = (first, start);
            loop {
                let quad = &self.faces[f];
                if quad.len() != 4 || !seen.insert(f) {
                    break;
                }
                faces.push(f);
                let Some(k) = (0..4).find(|&k| edge_key(quad[k], quad[(k + 1) % 4]) == entry)
                else {
                    break;
                };
                let exit = edge_key(quad[(k + 2) % 4], quad[(k + 3) % 4]);
                if exit == start {
                    break;
                }
                edges.push(exit);
                match adj.faces(exit[0], exit[1]) {
                    [x, y] => (f, entry) = (if *x == f { *y } else { *x }, exit),
                    _ => break,
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();
        (faces, edges)
    }

    /// Every vertex connected to `seeds` through edges (Blender `L`/`Ctrl+L`).
    pub fn linked(&self, seeds: &[u32]) -> Vec<u32> {
        let adj = self.adjacency();
        let n = self.positions.len();
        let mut on = vec![false; n];
        let mut stack: Vec<u32> = seeds
            .iter()
            .copied()
            .filter(|&v| (v as usize) < n)
            .collect();
        while let Some(v) = stack.pop() {
            if std::mem::replace(&mut on[v as usize], true) {
                continue;
            }
            stack.extend(
                adj.neighbors[v as usize]
                    .iter()
                    .filter(|w| !on[**w as usize]),
            );
        }
        indices(&on)
    }

    /// Selection grown by one ring: every vertex sharing a face or wire edge
    /// with a selected one (Blender `Ctrl+Numpad+`).
    pub fn grow_selection(&self, verts: &[u32]) -> Vec<u32> {
        let adj = self.adjacency();
        let n = self.positions.len();
        let mut on = mask(n, verts);
        for &v in verts.iter().filter(|v| (**v as usize) < n) {
            for &f in &adj.vertex_faces[v as usize] {
                for &w in &self.faces[f] {
                    on[w as usize] = true;
                }
            }
            for &w in &adj.neighbors[v as usize] {
                on[w as usize] = true;
            }
        }
        indices(&on)
    }

    /// Selection without its border: keeps vertices whose faces and wire
    /// neighbours are all selected (Blender `Ctrl+Numpad-`).
    pub fn shrink_selection(&self, verts: &[u32]) -> Vec<u32> {
        let adj = self.adjacency();
        let on = mask(self.positions.len(), verts);
        let inner = |v: usize| {
            adj.vertex_faces[v]
                .iter()
                .all(|&f| self.faces[f].iter().all(|w| on[*w as usize]))
                && adj.neighbors[v].iter().all(|w| on[*w as usize])
        };
        (0..on.len())
            .filter(|&v| on[v] && inner(v))
            .map(|v| v as u32)
            .collect()
    }

    /// Copies the selected vertices with the faces and edges lying entirely
    /// in the selection (Blender `Shift+D`). Returns the copies, in the
    /// order of `verts`.
    pub fn duplicate(&mut self, verts: &[u32]) -> Vec<u32> {
        let mut map: HashMap<u32, u32> = HashMap::new();
        let mut out = Vec::new();
        for &v in verts {
            if (v as usize) < self.positions.len() && !map.contains_key(&v) {
                let copy = self.add_vertex(self.positions[v as usize]);
                map.insert(v, copy);
                out.push(copy);
            }
        }
        let mut uvs = Vec::new();
        let faces: Vec<Vec<u32>> = self
            .faces
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let copy: Vec<u32> = f
                    .iter()
                    .map(|v| map.get(v).copied())
                    .collect::<Option<_>>()?;
                if let Some(uv) = self.uvs.get(i) {
                    uvs.push(uv.clone());
                }
                Some(copy)
            })
            .collect();
        let seams: Vec<[u32; 2]> = self
            .seams
            .iter()
            .filter_map(|&[a, b]| Some(edge_key(*map.get(&a)?, *map.get(&b)?)))
            .collect();
        let wire: Vec<[u32; 2]> = self
            .edges()
            .into_iter()
            .filter_map(|[a, b]| Some(edge_key(*map.get(&a)?, *map.get(&b)?)))
            .collect();
        self.faces.extend(faces);
        self.uvs.extend(uvs);
        self.seams.extend(seams);
        self.wire.extend(wire);
        self.normalize_wire();
        out
    }
}

fn mask(n: usize, verts: &[u32]) -> Vec<bool> {
    let mut on = vec![false; n];
    for &v in verts {
        if let Some(x) = on.get_mut(v as usize) {
            *x = true;
        }
    }
    on
}

fn indices(on: &[bool]) -> Vec<u32> {
    (0..on.len() as u32).filter(|&v| on[v as usize]).collect()
}
