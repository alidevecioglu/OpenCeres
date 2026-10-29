//! OpenCeres core: reading survey images, finding stars, mapping pixels to the
//! sky and writing MPC observation reports.
//!
//! Pixel coordinates are 0-based with (0, 0) at the centre of the first pixel
//! stored in the file. FITS uses 1-based coordinates, so FITS (1, 1) is our
//! (0, 0); the conversion happens only inside [`wcs`].

pub mod detect;
pub mod ephem;
pub mod fits;
pub mod known;
pub mod mpc;
pub mod orbit;
pub mod render;
pub mod time;
pub mod wcs;
