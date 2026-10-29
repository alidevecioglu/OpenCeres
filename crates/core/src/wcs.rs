//! World coordinate system: gnomonic (TAN) projection between pixels and the
//! sky, as described by the standard FITS WCS header keywords.
//!
//! Distortion terms (SIP, TPV) are not applied yet; they are small near the
//! reference pixel and our own plate solution will replace them.

use crate::fits::Header;

#[derive(Debug, Clone, PartialEq)]
pub struct Wcs {
    /// Reference pixel, FITS 1-based.
    pub crpix: [f64; 2],
    /// Sky position of the reference pixel: RA, Dec in degrees.
    pub crval: [f64; 2],
    /// Linear transformation from pixel offsets to intermediate coordinates, degrees/pixel.
    pub cd: [[f64; 2]; 2],
}

impl Wcs {
    /// Reads CRPIXn, CRVALn and either CDi_j, or PCi_j/CROTA2 with CDELTn.
    pub fn from_header(h: &Header) -> Option<Wcs> {
        let is_tan = |key| h.text(key).is_some_and(|t| t.ends_with("-TAN"));
        if !is_tan("CTYPE1") || !is_tan("CTYPE2") {
            return None;
        }
        let crpix = [h.float("CRPIX1")?, h.float("CRPIX2")?];
        let crval = [h.float("CRVAL1")?, h.float("CRVAL2")?];

        let cd = if let Some(cd11) = h.float("CD1_1") {
            [
                [cd11, h.float("CD1_2").unwrap_or(0.0)],
                [
                    h.float("CD2_1").unwrap_or(0.0),
                    h.float("CD2_2").unwrap_or(0.0),
                ],
            ]
        } else {
            let (d1, d2) = (h.float("CDELT1")?, h.float("CDELT2")?);
            if h.get("PC1_1").is_some() {
                // CDi_j = CDELTi × PCi_j
                let pc = |k| h.float(k);
                [
                    [
                        d1 * pc("PC1_1").unwrap_or(1.0),
                        d1 * pc("PC1_2").unwrap_or(0.0),
                    ],
                    [
                        d2 * pc("PC2_1").unwrap_or(0.0),
                        d2 * pc("PC2_2").unwrap_or(1.0),
                    ],
                ]
            } else {
                // Classic AIPS rotation (Calabretta & Greisen 2002, eq. 189).
                let (sin, cos) = h.float("CROTA2").unwrap_or(0.0).to_radians().sin_cos();
                [[d1 * cos, -d2 * sin], [d1 * sin, d2 * cos]]
            }
        };

        Some(Wcs { crpix, crval, cd })
    }

    /// 0-based pixel to (RA, Dec) in degrees, RA in [0, 360).
    pub fn pixel_to_sky(&self, x: f64, y: f64) -> (f64, f64) {
        let (dx, dy) = (x + 1.0 - self.crpix[0], y + 1.0 - self.crpix[1]);
        let xi = (self.cd[0][0] * dx + self.cd[0][1] * dy).to_radians();
        let eta = (self.cd[1][0] * dx + self.cd[1][1] * dy).to_radians();

        let (ra0, dec0) = (self.crval[0].to_radians(), self.crval[1].to_radians());
        let denom = dec0.cos() - eta * dec0.sin();
        let ra = ra0 + xi.atan2(denom);
        let dec = (dec0.sin() + eta * dec0.cos()).atan2(xi.hypot(denom));
        (ra.to_degrees().rem_euclid(360.0), dec.to_degrees())
    }

    /// (RA, Dec) in degrees to 0-based pixel; `None` for points on the far
    /// side of the sky, where the projection is undefined.
    pub fn sky_to_pixel(&self, ra: f64, dec: f64) -> Option<(f64, f64)> {
        let (ra0, dec0) = (self.crval[0].to_radians(), self.crval[1].to_radians());
        let (ra, dec) = (ra.to_radians(), dec.to_radians());
        let dra = ra - ra0;
        let cos_c = dec0.sin() * dec.sin() + dec0.cos() * dec.cos() * dra.cos();
        if cos_c <= 0.0 {
            return None;
        }
        let xi = (dec.cos() * dra.sin() / cos_c).to_degrees();
        let eta =
            ((dec0.cos() * dec.sin() - dec0.sin() * dec.cos() * dra.cos()) / cos_c).to_degrees();

        let [[a, b], [c, d]] = self.cd;
        let det = a * d - b * c;
        if det == 0.0 {
            return None;
        }
        let dx = (d * xi - b * eta) / det;
        let dy = (-c * xi + a * eta) / det;
        Some((dx + self.crpix[0] - 1.0, dy + self.crpix[1] - 1.0))
    }

    /// Mean pixel scale in arcseconds per pixel.
    pub fn pixel_scale(&self) -> f64 {
        let [[a, b], [c, d]] = self.cd;
        (a * d - b * c).abs().sqrt() * 3600.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fits;

    fn header(extra: &[&str]) -> Header {
        let mut cards = vec![
            "CTYPE1  = 'RA---TAN'",
            "CTYPE2  = 'DEC--TAN'",
            "CRPIX1  =               2000.5",
            "CRPIX2  =               1500.5",
            "CRVAL1  =         45.123456789",
            "CRVAL2  =         12.987654321",
        ];
        cards.extend_from_slice(extra);
        fits::parse(&fits::tests::make_fits(1, 1, &cards, &[0.0]))
            .unwrap()
            .header
    }

    fn pan_starrs_like() -> Wcs {
        // ~0.257″/px, slightly rotated, RA increasing to the left.
        Wcs::from_header(&header(&[
            "CD1_1   =     -7.1385E-05",
            "CD1_2   =      1.20E-06",
            "CD2_1   =      1.25E-06",
            "CD2_2   =      7.1390E-05",
        ]))
        .unwrap()
    }

    #[test]
    fn reference_pixel_maps_to_reference_coordinates() {
        let wcs = pan_starrs_like();
        let (ra, dec) = wcs.pixel_to_sky(1999.5, 1499.5);
        assert!((ra - 45.123456789).abs() < 1e-12);
        assert!((dec - 12.987654321).abs() < 1e-12);
    }

    #[test]
    fn round_trips_across_the_frame() {
        let wcs = pan_starrs_like();
        for &(x, y) in &[
            (0.0, 0.0),
            (3999.0, 0.0),
            (0.0, 2999.0),
            (3999.0, 2999.0),
            (1234.5, 987.25),
        ] {
            let (ra, dec) = wcs.pixel_to_sky(x, y);
            let (px, py) = wcs.sky_to_pixel(ra, dec).unwrap();
            assert!(
                (px - x).abs() < 1e-7 && (py - y).abs() < 1e-7,
                "{x},{y} -> {px},{py}"
            );
        }
        assert!((wcs.pixel_scale() - 0.257).abs() < 0.001);
    }

    #[test]
    fn handles_ra_wrap_at_zero() {
        let mut wcs = pan_starrs_like();
        wcs.crval = [0.001, 0.0];
        let (ra, _) = wcs.pixel_to_sky(3999.0, 1499.5); // ~0.14° to the west
        assert!(ra > 359.0, "ra {ra}");
    }

    #[test]
    fn cdelt_with_crota_matches_cd() {
        let wcs = Wcs::from_header(&header(&[
            "CDELT1  =         -7.0E-05",
            "CDELT2  =          7.0E-05",
            "CROTA2  =                 0.0",
        ]))
        .unwrap();
        assert_eq!(wcs.cd, [[-7.0e-5, 0.0], [0.0, 7.0e-5]]);
    }

    #[test]
    fn crota_uses_each_axis_scale_in_its_own_term() {
        // CD1_2 = -CDELT2 sin ρ, CD2_1 = CDELT1 sin ρ.
        let wcs = Wcs::from_header(&header(&[
            "CDELT1  =         -7.0E-05",
            "CDELT2  =          8.0E-05",
            "CROTA2  =                 30.0",
        ]))
        .unwrap();
        let (sin, cos) = 30f64.to_radians().sin_cos();
        let expected = [
            [-7.0e-5 * cos, -8.0e-5 * sin],
            [-7.0e-5 * sin, 8.0e-5 * cos],
        ];
        for (got, want) in wcs.cd.iter().flatten().zip(expected.iter().flatten()) {
            assert!((got - want).abs() < 1e-18, "{got} vs {want}");
        }
    }

    #[test]
    fn rejects_non_tan_projection() {
        let img = fits::parse(&fits::tests::make_fits(
            1,
            1,
            &["CTYPE1  = 'RA---SIN'", "CTYPE2  = 'DEC--SIN'"],
            &[0.0],
        ))
        .unwrap();
        assert!(Wcs::from_header(&img.header).is_none());
    }
}
