//! Read-only Bruker TDF frames, TSF line spectra and miniTDF spectra, with
//! source metadata and without spectrum synthesis.
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
#![warn(missing_docs)]
// Documented module by module; `missing_docs` warns everywhere else.
mod compression;
#[allow(missing_docs)]
mod database;
mod error;
#[allow(missing_docs)]
mod mass;
#[cfg(feature = "minitdf")]
#[allow(missing_docs)]
mod minitdf;
#[cfg(feature = "minitdf")]
mod minitdf_blob;
#[allow(missing_docs)]
mod mobility;
#[allow(missing_docs)]
mod reader;
mod schema;
#[allow(missing_docs)]
mod tables;
#[allow(missing_docs)]
mod tsf;
mod tsf_blob;

pub use database::{MetadataDatabase, MetadataLimits, MetadataRow, SqlValue};
pub use error::{Error, ErrorContext, ErrorKind};
pub(crate) use error::{field, invalid, limit};
pub use mass::{LinearMzScale, MzCalibration, MzModel};
#[cfg(feature = "minitdf")]
pub use minitdf::{MiniTdfPrecursor, MiniTdfReader, MiniTdfSpectrum};
pub use mobility::{LinearMobilityScale, MobilityModel};
pub use reader::TdfReader;
pub use tables::{
    AcquisitionType, DiaFrameMsMsInfo, DiaFrameMsMsWindow, DiaFrameMsMsWindowGroup, FrameInfo,
    GlobalMetadata, LineSpectrum, PasefFrameMsMsInfo, Polarity, Precursor, TimsCalibration,
    TsfFrameInfo, TsfFrameMsMsInfo,
};
pub use tsf::TsfReader;

/// The result of every fallible call in this crate: see [`Error`] for the
/// failure variants and [`ErrorKind`] for the category to match on.
pub type Result<T> = std::result::Result<T, Error>;

/// Resource ceilings checked before allocating compressed or decoded frame buffers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Largest compressed frame or spectrum blob read from the binary file, in
    /// bytes. A header claiming more is rejected before the read.
    pub max_compressed_bytes: usize,
    /// Largest decoded blob, in bytes. It is also the Zstandard decoder's
    /// window ceiling, so a decompression bomb is refused, not buffered.
    pub max_decompressed_bytes: usize,
    /// Largest peak count accepted for one frame or spectrum.
    pub max_peaks: usize,
    /// Largest scan count accepted for one frame, including the count an LZF
    /// frame header carries, which may exceed `Frames.NumScans`.
    pub max_scans: usize,
    /// Ceilings for the SQLite metadata: rows, columns, value and result
    /// sizes, virtual-machine steps and the query timeout.
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

/// Scratch buffers a caller keeps across reads: the compressed bytes read from
/// the binary file and the bytes the Zstandard decoder writes. Reading frame
/// after frame, or spectrum after spectrum, through one `DecodeBuffers`
/// allocates once instead of per read, which is what
/// [`TdfReader::read_frame_into_buffers`], [`TsfReader::read_line_spectrum_into_buffers`]
/// and `MiniTdfReader::read_spectrum_into_buffers` are for. Both buffers are
/// cleared by each read and keep their capacity; after an error they hold
/// partial data, which the next read discards.
///
/// One set belongs to one reading thread. A reader is `Send + Sync` and takes
/// `&self`, so give each thread its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeBuffers {
    /// The compressed frame or spectrum, as read from the binary file.
    pub compressed: Vec<u8>,
    /// What the Zstandard decoder wrote. Empty for an LZF frame, which is
    /// decoded straight into the frame vectors.
    pub decoded: Vec<u8>,
}

/// Original raw coordinates and counts. Intensities are not rescaled.
/// `Default` gives the empty frame a buffer-reusing reader fills.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// `Frames.Id` of the frame these coordinates were read from.
    pub id: u64,
    /// Where each scan starts in `tof_indices` and `intensities`, with a final
    /// entry equal to the peak count: scan `i` spans
    /// `scan_offsets[i]..scan_offsets[i + 1]`.
    pub scan_offsets: Vec<usize>,
    /// Time-of-flight index of every peak, scan by scan and ascending within a
    /// scan. Convert with [`MzModel`] or [`LinearMzScale`].
    pub tof_indices: Vec<u32>,
    /// Intensity of every peak, in the same order, as stored.
    pub intensities: Vec<u32>,
}

impl Frame {
    /// Scans stored in the binary frame. This is `Frames.NumScans` for Zstandard
    /// frames; LZF frames may store more.
    pub fn scan_count(&self) -> usize {
        self.scan_offsets.len().saturating_sub(1)
    }
}
