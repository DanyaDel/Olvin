//! Fuzzy matching for quick open and the command palette: the query's chars
//! must appear in order; consecutive runs, word starts and hits in the file
//! name score higher.

/// Score of `candidate` for `query` (higher is better), `None` if it doesn't
/// match. Case-insensitive. Also returns the matched char positions.
pub fn score(query: &str, candidate: &str) -> Option<(i32, Vec<usize>)> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    let c: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = c
        .iter()
        .map(|ch| ch.to_lowercase().next().unwrap_or(*ch))
        .collect();
    // The file name starts after the last separator.
    let name_start = c
        .iter()
        .rposition(|&ch| ch == '/' || ch == '\\')
        .map_or(0, |p| p + 1);

    // Try every start of the first char; keep the best greedy match.
    let mut best: Option<(i32, Vec<usize>)> = None;
    for start in (0..lower.len()).filter(|&i| lower[i] == q[0]) {
        let Some(positions) = greedy(&lower, &q, start) else {
            break; // later starts can't match either
        };
        let s = rate(&c, &positions, name_start);
        if best.as_ref().is_none_or(|(b, _)| s > *b) {
            best = Some((s, positions));
        }
    }
    best
}

fn greedy(lower: &[char], q: &[char], start: usize) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(q.len());
    let mut i = start;
    for &qc in q {
        while i < lower.len() && lower[i] != qc {
            i += 1;
        }
        if i == lower.len() {
            return None;
        }
        positions.push(i);
        i += 1;
    }
    Some(positions)
}

fn rate(c: &[char], positions: &[usize], name_start: usize) -> i32 {
    let mut s = 0;
    for (k, &p) in positions.iter().enumerate() {
        s += 1;
        if k > 0 && positions[k - 1] + 1 == p {
            s += 6;
        }
        let prev = p.checked_sub(1).map(|i| c[i]);
        let word_start = match prev {
            None => true,
            Some(pc) => !pc.is_alphanumeric() || (pc.is_lowercase() && c[p].is_uppercase()),
        };
        if word_start {
            s += 8;
        }
        if p >= name_start {
            s += 3;
        }
    }
    // Shorter candidates and earlier matches win ties.
    s * 4 - (c.len() as i32) / 4 - positions[0] as i32 / 8
}

/// Sorts `items` by score for `query`, dropping non-matches.
pub fn rank<'a, 'b, T>(
    query: &str,
    items: &'a [T],
    text: impl Fn(&'a T) -> &'b str,
) -> Vec<(&'a T, Vec<usize>)> {
    let mut scored: Vec<(i32, &T, Vec<usize>)> = items
        .iter()
        .filter_map(|it| score(query, text(it)).map(|(s, p)| (s, it, p)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| text(a.1).cmp(text(b.1))));
    scored.into_iter().map(|(_, it, p)| (it, p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_in_order() {
        assert!(score("mrs", "src/main.rs").is_some());
        assert!(score("MAIN", "src/main.rs").is_some());
        assert!(score("srm", "src/main.rs").is_some());
        assert!(score("zz", "src/main.rs").is_none());
        assert!(score("sm", "ms").is_none());
        assert_eq!(score("", "x").map(|s| s.0), Some(0));
        let (_, pos) = score("main", "src/main.rs").unwrap();
        assert_eq!(pos, vec![4, 5, 6, 7]);
    }

    #[test]
    fn ranks_sensibly() {
        let files = [
            "assets/shaders/water.wgsl",
            "src/main.rs",
            "src/player/movement.rs",
            "assets/scenes/main.scene.ron",
            "Cargo.toml",
        ];
        let ranked: Vec<&str> = rank("main", &files, |s| s)
            .into_iter()
            .map(|(s, _)| *s)
            .collect();
        assert_eq!(ranked[0], "src/main.rs");
        assert_eq!(ranked.len(), 2);
        assert_eq!(
            rank("water", &files, |s| s)[0].0,
            &"assets/shaders/water.wgsl"
        );
        assert_eq!(rank("pm", &files, |s| s)[0].0, &"src/player/movement.rs");
        assert_eq!(rank("ct", &files, |s| s)[0].0, &"Cargo.toml");
        assert_eq!(rank("", &files, |s| s).len(), files.len());
        // Word starts beat scattered letters.
        let cmds = ["Toggle Comment", "Go to Line", "Close Tab"];
        assert_eq!(rank("tc", &cmds, |s| s)[0].0, &"Toggle Comment");
        assert_eq!(rank("gtl", &cmds, |s| s)[0].0, &"Go to Line");
    }
}
