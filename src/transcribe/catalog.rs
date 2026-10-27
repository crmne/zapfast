//! A small, pinned catalog. No Hugging Face code is downloaded or executed.

use serde::{Deserialize, Serialize};

/// The models offered in Settings. Existing settings default to the full
/// precision v3 that was compared with the system recognizer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Model {
    #[default]
    ParakeetV3,
    ParakeetV3Int8,
    ParakeetUltra,
}

impl Model {
    pub const ALL: [Self; 3] = [Self::ParakeetV3, Self::ParakeetV3Int8, Self::ParakeetUltra];

    pub fn label(self) -> &'static str {
        match self {
            Self::ParakeetV3 => "Parakeet v3 · FP32",
            Self::ParakeetV3Int8 => "Parakeet v3 · INT8",
            Self::ParakeetUltra => "Parakeet Ultra · FP32",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ParakeetV3 => "Full precision. Recommended for transcription quality.",
            Self::ParakeetV3Int8 => "Uses less memory, but can omit words and be less accurate.",
            Self::ParakeetUltra => "A refined version of v3. Results vary with the recording.",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::ParakeetV3 => "parakeet-v3-fp32",
            Self::ParakeetV3Int8 => "parakeet-v3-int8",
            Self::ParakeetUltra => "parakeet-ultra-fp32",
        }
    }

    pub fn repository(self) -> &'static str {
        match self {
            Self::ParakeetV3 | Self::ParakeetV3Int8 => "istupakov/parakeet-tdt-0.6b-v3-onnx",
            Self::ParakeetUltra => "Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx",
        }
    }

    pub fn revision(self) -> &'static str {
        match self {
            Self::ParakeetV3 | Self::ParakeetV3Int8 => "8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce",
            Self::ParakeetUltra => "b27f93c2238aa35e138996c005a62f1e8430f801",
        }
    }

    /// Stored in the encrypted archive, never inferred from the current
    /// setting when an older job finishes.
    pub fn engine(self) -> String {
        format!("{}/{}/native-1", self.key(), &self.revision()[..8])
    }

    pub fn total_bytes(self) -> u64 {
        self.assets().iter().map(|asset| asset.bytes).sum()
    }

    pub(super) fn assets(self) -> &'static [Asset] {
        match self {
            Self::ParakeetV3 => V3,
            Self::ParakeetV3Int8 => V3_INT8,
            Self::ParakeetUltra => ULTRA,
        }
    }

    pub(super) fn directory(self, root: &std::path::Path) -> std::path::PathBuf {
        root.join(self.key()).join(self.revision())
    }

    pub(super) fn url(self, asset: &Asset) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repository(),
            self.revision(),
            asset.remote
        )
    }
}

pub(super) struct Asset {
    pub remote: &'static str,
    pub local: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

const VOCAB: Asset = Asset {
    remote: "vocab.txt",
    local: "vocab.txt",
    bytes: 93_939,
    sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
};

const V3: &[Asset] = &[
    Asset {
        remote: "encoder-model.onnx",
        local: "encoder-model.onnx",
        bytes: 41_770_866,
        sha256: "98a74b21b4cc0017c1e7030319a4a96f4a9506e50f0708f3a516d02a77c96bb1",
    },
    Asset {
        remote: "encoder-model.onnx.data",
        local: "encoder-model.onnx.data",
        bytes: 2_435_420_160,
        sha256: "9a22d372c51455c34f13405da2520baefb7125bd16981397561423ed32d24f36",
    },
    Asset {
        remote: "decoder_joint-model.onnx",
        local: "decoder_joint-model.onnx",
        bytes: 72_520_893,
        sha256: "e978ddf6688527182c10fde2eb4b83068421648985ef23f7a86be732be8706c1",
    },
    VOCAB,
];

const V3_INT8: &[Asset] = &[
    Asset {
        remote: "encoder-model.int8.onnx",
        local: "encoder-model.int8.onnx",
        bytes: 652_183_999,
        sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
    },
    Asset {
        remote: "decoder_joint-model.int8.onnx",
        local: "decoder_joint-model.int8.onnx",
        bytes: 18_202_004,
        sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
    },
    VOCAB,
];

const ULTRA: &[Asset] = &[
    Asset {
        remote: "fp32/encoder-model.onnx",
        local: "encoder-model.onnx",
        bytes: 503_828,
        sha256: "af8d232c654c696eb07a3cf9fa164a9649209b759409e6ec9913f8155d255a23",
    },
    Asset {
        remote: "fp32/encoder-model.onnx.data.000",
        local: "encoder-model.onnx.data.000",
        bytes: 1_483_313_152,
        sha256: "2762ecdbc59be479843dca0140e49300fd6e98ec994e19d0a7003308f0b3282d",
    },
    Asset {
        remote: "fp32/encoder-model.onnx.data.001",
        local: "encoder-model.onnx.data.001",
        bytes: 952_109_056,
        sha256: "d26ce092d3291e893f79794cb06306f6d18390dee332330d68fc4b096b4762f3",
    },
    Asset {
        remote: "fp32/decoder_joint-model.onnx",
        local: "decoder_joint-model.onnx",
        bytes: 9_296,
        sha256: "7fa05c5183dfc89dbb0f34d4b2e7935f877762f2438bbcc79729f4e2d00eb041",
    },
    Asset {
        remote: "fp32/decoder_joint-model.onnx.data",
        local: "decoder_joint-model.onnx.data",
        bytes: 72_514_072,
        sha256: "5d6873a9bf49b704270a3e3dcc6c71090e1edd3248c6499b5d3cadd9f23dbcec",
    },
    VOCAB,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_is_pinned_and_never_runs_remote_code() {
        for model in Model::ALL {
            assert_eq!(model.revision().len(), 40);
            assert!(!model.assets().is_empty());
            for asset in model.assets() {
                assert_eq!(asset.sha256.len(), 64);
                assert!(asset.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()));
                assert!(asset.bytes > 0);
                assert!(!asset.local.contains(['/', '\\']));
                assert!(!asset.remote.contains(".."));
                assert!(model.url(asset).starts_with("https://huggingface.co/"));
                assert!(!asset.remote.ends_with(".py"));
            }
        }
        assert_eq!(Model::default(), Model::ParakeetV3);
        assert!(Model::ParakeetV3Int8.total_bytes() < Model::ParakeetV3.total_bytes());
    }
}
