//! Looks (`*.look.ron`): a named picture filter, i.e. the post-processing,
//! tonemapping and fog part of an [`Environment`], applied over a scene without
//! touching its lighting or sky. Paths (the LUT) are relative to the project's
//! `assets/`, like everywhere else.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::file::{AoDesc, BloomDesc, Environment, FogDesc, HeightFogDesc};

/// A picture look. Every field has a serde default (the neutral value), so a
/// file only lists what it changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LookDesc {
    pub name: String,
    /// Camera exposure, a multiplier.
    pub exposure: f32,
    /// ACES filmic tonemapping; off = plain clamp.
    pub filmic: bool,
    /// Glow around bright areas (Full tier).
    pub bloom: Option<BloomDesc>,
    /// FXAA anti-aliasing (Full tier).
    pub fxaa: bool,
    /// Ambient occlusion (Full tier).
    pub ao: Option<AoDesc>,
    /// 1 = unchanged, 0 = grey.
    pub saturation: f32,
    /// Around mid grey; 1 = unchanged.
    pub contrast: f32,
    pub lift: [f32; 3],
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    /// Colour grading LUT strip, path relative to `assets/` (Full tier).
    pub lut: Option<String>,
    pub lut_strength: f32,
    /// Distance fog of the look; `None` keeps the scene's own fog.
    pub fog: Option<FogDesc>,
    /// Height fog of the look; `None` keeps the scene's own.
    pub height_fog: Option<HeightFogDesc>,
}

impl Default for LookDesc {
    /// The neutral look: no grade, no bloom, FXAA on, ACES.
    fn default() -> Self {
        Self {
            name: "Neutral".into(),
            exposure: 1.0,
            filmic: true,
            bloom: None,
            fxaa: true,
            ao: None,
            saturation: 1.0,
            contrast: 1.0,
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
            lut: None,
            lut_strength: 1.0,
            fog: None,
            height_fog: None,
        }
    }
}

impl LookDesc {
    /// Sets the look's fields on `env`; lighting, sky and shadows stay as they are.
    pub fn apply_to(&self, env: &mut Environment) {
        env.exposure = self.exposure;
        env.filmic = self.filmic;
        if self.fog.is_some() {
            env.fog = self.fog;
        }
        if self.height_fog.is_some() {
            env.height_fog = self.height_fog;
        }
        let post = &mut env.post;
        post.fxaa = self.fxaa;
        post.bloom = self.bloom;
        post.ao = self.ao;
        post.saturation = self.saturation;
        post.contrast = self.contrast;
        post.lift = self.lift;
        post.gamma = self.gamma;
        post.gain = self.gain;
        post.lut = self.lut.clone();
        post.lut_strength = self.lut_strength;
    }

    /// The look `env` has now (fog is kept only when the scene has it on).
    pub fn from_environment(name: &str, env: &Environment) -> Self {
        let post = &env.post;
        Self {
            name: name.to_owned(),
            exposure: env.exposure,
            filmic: env.filmic,
            bloom: post.bloom,
            fxaa: post.fxaa,
            ao: post.ao,
            saturation: post.saturation,
            contrast: post.contrast,
            lift: post.lift,
            gamma: post.gamma,
            gain: post.gain,
            lut: post.lut.clone(),
            lut_strength: post.lut_strength,
            fog: env.fog,
            height_fog: env.height_fog,
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, self.to_ron()).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn to_ron(&self) -> String {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().struct_names(false))
            .expect("a look serialises")
    }

    /// The looks that ship in the editor and need no files (the
    /// `looks-cinematic` library pack carries the same ones as assets; Retro
    /// there also uses a posterising LUT).
    pub fn built_in() -> Vec<LookDesc> {
        let look = |name: &str, f: &dyn Fn(&mut LookDesc)| {
            let mut l = LookDesc {
                name: name.into(),
                ..Default::default()
            };
            f(&mut l);
            l
        };
        let bloom = |threshold, intensity| {
            Some(BloomDesc {
                threshold,
                intensity,
            })
        };
        vec![
            look("Neutral", &|_| {}),
            look("Cinematic", &|l| {
                l.bloom = bloom(1.0, 0.25);
                l.contrast = 1.08;
                l.saturation = 1.05;
                // Teal shadows, orange highlights.
                l.lift = [-0.01, 0.005, 0.02];
                l.gain = [1.07, 1.0, 0.9];
                l.gamma = [1.0, 1.0, 1.05];
            }),
            look("Noir", &|l| {
                l.saturation = 0.0;
                l.contrast = 1.35;
                l.gamma = [0.9; 3];
                l.lift = [-0.02; 3];
                l.bloom = bloom(1.4, 0.12);
            }),
            look("Vibrant", &|l| {
                l.saturation = 1.35;
                l.contrast = 1.1;
                l.gain = [1.04, 1.04, 1.04];
                l.bloom = bloom(1.1, 0.18);
            }),
            look("Retro PS1", &|l| {
                l.fxaa = false;
                l.saturation = 0.7;
                l.contrast = 1.15;
                l.gamma = [1.05, 1.0, 0.95];
            }),
            look("Night Vision", &|l| {
                l.exposure = 1.3;
                l.saturation = 0.0;
                l.contrast = 1.2;
                l.gain = [0.35, 1.25, 0.4];
                l.lift = [0.0, 0.02, 0.0];
                l.bloom = bloom(0.7, 0.45);
            }),
            look("Dreamy", &|l| {
                l.bloom = bloom(0.6, 0.6);
                l.contrast = 0.85;
                l.saturation = 1.1;
                l.lift = [0.06, 0.05, 0.08];
                l.gain = [1.0, 0.97, 1.03];
            }),
            look("Horror", &|l| {
                l.exposure = 0.85;
                l.saturation = 0.45;
                l.contrast = 1.2;
                l.gain = [0.85, 0.97, 1.05];
                l.gamma = [0.95, 1.0, 1.05];
                l.height_fog = Some(HeightFogDesc {
                    color: [0.1, 0.13, 0.16],
                    density: 0.12,
                    falloff: 0.35,
                    height: 0.0,
                });
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_look_matches_the_default_environment() {
        let mut env = Environment::default();
        LookDesc::default().apply_to(&mut env);
        assert!(env.is_default());
    }

    #[test]
    fn looks_touch_only_their_fields_and_round_trip() {
        let mut env = (Environment::PRESETS[1].1)();
        let before = env.clone();
        for look in LookDesc::built_in() {
            let mut e = before.clone();
            look.apply_to(&mut e);
            assert_eq!(
                (
                    &e.sky,
                    &e.ground,
                    &e.background,
                    &e.skybox,
                    e.shadow_distance
                ),
                (
                    &before.sky,
                    &before.ground,
                    &before.background,
                    &before.skybox,
                    before.shadow_distance
                ),
                "{} changed lighting",
                look.name
            );
            // Applying what the scene now has gives the same look back.
            let back = LookDesc::from_environment(&look.name, &e);
            let mut again = before.clone();
            back.apply_to(&mut again);
            assert_eq!(e, again, "{}", look.name);
            let parsed: LookDesc = ron::from_str(&look.to_ron()).unwrap();
            assert_eq!(parsed, look);
        }
        env = before;
        assert_ne!(env, {
            let mut e = env.clone();
            LookDesc::built_in()[2].apply_to(&mut e);
            e
        });
    }

    #[test]
    fn a_partial_file_defaults_the_rest_and_typos_are_rejected() {
        let look: LookDesc = ron::from_str("(name: \"Dim\", exposure: 0.5)").unwrap();
        assert_eq!(look.exposure, 0.5);
        assert!(look.fxaa && look.filmic && look.bloom.is_none());
        assert!(ron::from_str::<LookDesc>("(expsure: 0.5)").is_err());
    }

    #[test]
    fn built_in_names_are_unique() {
        let looks = LookDesc::built_in();
        assert_eq!(looks.len(), 8);
        let mut names: Vec<_> = looks.iter().map(|l| l.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 8);
    }

    /// The looks of the `looks-cinematic` library pack by file name: the
    /// built-in ones, except that Retro PS1 and Night Vision grade through a
    /// LUT strip of the pack instead of the simple controls.
    fn pack_files() -> Vec<(String, LookDesc)> {
        LookDesc::built_in()
            .into_iter()
            .map(|mut l| {
                let file = l.name.to_lowercase().replace(' ', "_");
                match l.name.as_str() {
                    "Retro PS1" => {
                        l.lut = Some("looks-cinematic/luts/retro_ps1.png".into());
                        l.saturation = 1.0;
                        l.contrast = 1.1;
                        l.gamma = [1.0; 3];
                    }
                    "Night Vision" => {
                        l.lut = Some("looks-cinematic/luts/night_vision.png".into());
                        l.saturation = 1.0;
                        l.contrast = 1.0;
                        l.gain = [1.0; 3];
                        l.lift = [0.0; 3];
                    }
                    _ => {}
                }
                (format!("{file}.look.ron"), l)
            })
            .collect()
    }

    #[test]
    fn the_looks_pack_files_match_the_code() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../library/looks-cinematic/assets/looks-cinematic/looks");
        // `OLVIN_WRITE_LOOKS=1 cargo test -p olvin_scene --features serde looks_pack`
        // rewrites them.
        if std::env::var_os("OLVIN_WRITE_LOOKS").is_some() {
            for (file, look) in pack_files() {
                look.save(dir.join(file)).unwrap();
            }
        }
        for (file, look) in pack_files() {
            let on_disk = LookDesc::load(dir.join(&file)).unwrap();
            assert_eq!(on_disk, look, "{file} drifted from the code");
        }
        let count = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(count, pack_files().len());
    }
}
