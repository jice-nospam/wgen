use serde::{Deserialize, Serialize};

use super::normalize;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct NormalizeConf {
    pub min: f32,
    pub max: f32,
}

impl Default for NormalizeConf {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0 }
    }
}

pub fn gen_normalize(hmap: &mut [f32], conf: &NormalizeConf) {
    normalize(hmap, conf.min, conf.max);
}
