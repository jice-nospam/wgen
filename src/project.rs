use std::path::Path;

use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

use crate::worldgen::Step;
use crate::VERSION;

/// The content of a `.wgen` project file: the seed and the step stack, nothing else.
///
/// Compatibility policy:
/// - a file written by an older wgen loads; fields it lacks take their `#[serde(default)]`,
///   fields it has that no longer exist are ignored (serde's default);
/// - a file written by a newer wgen is refused, its content may mean something else;
/// - the `version` field records the writer, it is not part of the data.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// wgen version that wrote the file; empty in files older than the field
    #[serde(default)]
    pub version: String,
    /// random number generator's seed
    pub seed: u64,
    /// generator steps with their configuration and masks
    pub steps: Vec<Step>,
}

impl Project {
    pub fn new(seed: u64, steps: Vec<Step>) -> Self {
        Self {
            version: VERSION.to_owned(),
            seed,
            steps,
        }
    }

    pub fn load(file_path: &str) -> Result<Self, String> {
        let contents = std::fs::read_to_string(file_path)
            .map_err(|e| format!("Unable to read the file : {}", e))?;
        Self::from_ron(&contents)
    }

    pub fn save(&self, file_path: &str) -> Result<(), String> {
        let data = self.to_ron()?;
        std::fs::write(Path::new(file_path), data)
            .map_err(|e| format!("Unable to write the file : {}", e))
    }

    fn from_ron(contents: &str) -> Result<Self, String> {
        let project: Project =
            ron::from_str(contents).map_err(|e| format!("Cannot parse the file : {}", e))?;
        check_version(&project.version)?;
        Ok(project)
    }

    fn to_ron(&self) -> Result<String, String> {
        // one step per line, masks kept on a single line
        let config = PrettyConfig::new().compact_arrays(true);
        ron::ser::to_string_pretty(self, config).map_err(|e| format!("Cannot serialize : {}", e))
    }
}

/// refuses a file written by a wgen newer than this build
fn check_version(file_version: &str) -> Result<(), String> {
    if file_version.is_empty() {
        return Ok(());
    }
    let file = parse_version(file_version)
        .ok_or_else(|| format!("Unreadable file version '{}'", file_version))?;
    let build = parse_version(VERSION).expect("CARGO_PKG_VERSION is semver");
    if file > build {
        return Err(format!(
            "This file was saved by wgen {}, newer than this wgen {}",
            file_version, VERSION
        ));
    }
    Ok(())
}

/// `major.minor.patch` as a comparable tuple; a pre-release suffix is ignored
fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next()??;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::{HillsConf, NormalizeConf};
    use crate::worldgen::StepType;
    use crate::MASK_SIZE;

    #[test]
    fn version_ordering() {
        assert_eq!(parse_version("0.3.1"), Some((0, 3, 1)));
        assert_eq!(parse_version("1.2.3-beta"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.3"), None);
        assert!(check_version("").is_ok());
        assert!(check_version("0.0.1").is_ok());
        assert!(check_version(VERSION).is_ok());
        assert!(check_version("999.0.0").is_err());
        assert!(check_version("garbage").is_err());
    }

    #[test]
    fn old_whole_panel_format_loads() {
        // written by wgen 0.3.1 : the whole generator panel, widget state included
        let old = r#"(version:"0.3.1",steps:[(disabled:false,mask:None,typ:Hills((nb_hill:600,base_radius:16.0,radius_var:0.7,height:0.3))),(disabled:true,mask:None,typ:Normalize((min:0.0,max:1.0)))],cur_step:(disabled:false,mask:None,typ:MudSlide((iterations:5.0,max_erosion_alt:0.9,strength:0.4,water_level:0.12))),selected_step:1,move_to_pos:0,hovered:false,seed:3735928559)"#;
        let project = Project::from_ron(old).unwrap();
        assert_eq!(project.seed, 0xdeadbeef);
        assert_eq!(project.steps.len(), 2);
        assert!(project.steps[1].disabled);
        assert_eq!(project.steps[0].mask_feather, 0.0);
    }

    #[test]
    fn file_without_version_loads() {
        let project = Project::from_ron("(seed:1,steps:[])").unwrap();
        assert_eq!(project.version, "");
        assert_eq!(project.seed, 1);
    }

    #[test]
    fn round_trip_keeps_masks_and_stays_compact() {
        let mut mask = vec![1.0; MASK_SIZE * MASK_SIZE];
        mask[7] = 0.25;
        let project = Project::new(
            42,
            vec![
                Step {
                    typ: StepType::Hills(HillsConf::default()),
                    mask: Some(mask),
                    mask_feather: 0.3,
                    ..Default::default()
                },
                Step {
                    typ: StepType::Normalize(NormalizeConf::default()),
                    disabled: true,
                    ..Default::default()
                },
            ],
        );
        let text = project.to_ron().unwrap();
        // a 64x64 mask must not become 4096 lines
        let mask_lines: Vec<&str> = text.lines().filter(|l| l.contains("mask: Some(")).collect();
        assert_eq!(mask_lines.len(), 1);
        assert!(mask_lines[0].contains("0.25"));
        // the feather sits on its own line right after the mask, one field per line
        let lines: Vec<&str> = text.lines().collect();
        let mask_at = lines.iter().position(|l| l.contains("mask: Some(")).unwrap();
        assert_eq!(lines[mask_at + 1].trim(), "mask_feather: 0.3,");
        assert!(text.lines().count() < 40, "{}", text);
        assert_eq!(Project::from_ron(&text).unwrap(), project);
    }

    #[test]
    fn tracked_examples_load() {
        for name in ["ex_continent.wgen", "ex_hills.wgen", "ex_island.wgen"] {
            let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), name);
            let project = Project::load(&path).unwrap_or_else(|e| panic!("{}: {}", name, e));
            assert!(!project.steps.is_empty(), "{} has no steps", name);
            assert_eq!(project.version, VERSION, "{} needs re-stamping", name);
        }
    }
}
