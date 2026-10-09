//! Read-only TDF frames and source metadata without spectrum synthesis.
//!
//! Frame identifiers, scan offsets, TOF indices and detector intensities are retained.
//! Calibration is explicit and unsupported source models return an error.
//!
//! ```no_run
//! use sage_plus_tdf::{Result, TdfReader};
//!
//! fn inspect(path: &str) -> Result<()> {
//!     let reader = TdfReader::open(path)?;
//!     let ids: Vec<_> = reader.frames().map(|frame| frame.id).collect();
//!     for id in ids {
//!         let frame = reader.read_frame(id)?;
//!         for (scan, offsets) in frame.scan_offsets.windows(2).enumerate() {
//!             let ions = offsets[0]..offsets[1];
//!             println!("frame {id}, scan {scan}: {} ions", ions.len());
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! Mass and mobility models are loaded from the frame's recorded references.
//! Missing calibration and unsupported equations are reported explicitly.
//!
//! ```no_run
//! # use sage_plus_tdf::{Result, TdfReader};
//! # fn calibrated(reader: &TdfReader, id: u64) -> Result<()> {
//! let mz = reader.mz_model(id)?.mz(100_000.0)?;
//! let mobility = reader.mobility_model(id)?.one_over_k0(100.0)?;
//! println!("m/z {mz}, inverse reduced mobility {mobility}");
//! # Ok(())
//! # }
//! ```
#![forbid(unsafe_code)]
mod compression;
mod database;
mod error;
mod mass;
mod mobility;
mod reader;
mod schema;
mod tables;

pub use database::{MetadataDatabase, MetadataLimits, MetadataRow, SqlValue};
pub use error::{Error, ErrorContext, ErrorKind};
pub(crate) use error::{field, invalid, limit};
pub use mass::{LinearMzScale, MzCalibration, MzModel};
pub use mobility::{LinearMobilityScale, MobilityModel};
pub use reader::TdfReader;
pub use tables::{
    AcquisitionType, DiaFrameMsMsInfo, DiaFrameMsMsWindow, DiaFrameMsMsWindowGroup, FrameInfo,
    GlobalMetadata, PasefFrameMsMsInfo, Precursor, TimsCalibration,
};

pub type Result<T> = std::result::Result<T, Error>;

/// Resource ceilings checked before allocating compressed or decoded frame buffers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_compressed_bytes: usize,
    pub max_decompressed_bytes: usize,
    pub max_peaks: usize,
    pub max_scans: usize,
    pub metadata: MetadataLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_compressed_bytes: 256 * 1024 * 1024,
            max_decompressed_bytes: 512 * 1024 * 1024,
            max_peaks: 32_000_000,
            max_scans: 100_000,
            metadata: MetadataLimits::default(),
        }
    }
}

/// Original raw coordinates and counts. Intensities are not rescaled.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: u64,
    pub scan_offsets: Vec<usize>,
    pub tof_indices: Vec<u32>,
    pub intensities: Vec<u32>,
}

impl Frame {
    /// Scans stored in the binary frame. This is `Frames.NumScans` for Zstandard
    /// frames; LZF frames may store more.
    pub fn scan_count(&self) -> usize {
        self.scan_offsets.len().saturating_sub(1)
    }
}
