//! Gaia DR3 reference stars from VizieR (catalogue `I/355/gaiadr3`), for
//! plate solutions and photometry.
//!
//! The core only builds the request and parses the reply (VizieR's ASU
//! tab-separated output, <https://vizier.cds.unistra.fr/doc/asu-summary.htx>);
//! fetching is up to the caller, so this module stays dependency-free.
//!
//! `RA_ICRS`/`DE_ICRS` in `I/355/gaiadr3` are at epoch J2016.0 (the table's
//! `#Coosys H_2016.000: ICRS` line and the column notes say `Ep=2016.0`), so
//! every position is carried to the observation date with the star's proper
//! motion ([`GaiaStar::position_at`]).
//!
//! Mirrors don't fail cleanly: on a test of all five (2026-11-17) one
//! answered with an HTML "page unavailable", one with `200 OK` and
//! `#INFO Error=…` or a `no connection` report, one with `200 OK` and an
//! empty table. A reply therefore only counts when it parses *and* holds
//! Gaia rows; see [`parse_tsv`].

/// A VizieR mirror: a short name for Settings and its base URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mirror {
    pub name: &'static str,
    pub url: &'static str,
}

/// Known VizieR mirrors, CDS (the main site) first.
pub const MIRRORS: [Mirror; 5] = [
    Mirror {
        name: "CDS Strasbourg",
        url: "https://vizier.cds.unistra.fr",
    },
    Mirror {
        name: "CfA Harvard",
        url: "https://vizier.cfa.harvard.edu",
    },
    Mirror {
        name: "IDIA Cape Town",
        url: "https://vizier.idia.ac.za",
    },
    Mirror {
        name: "NAOJ Tokyo",
        url: "http://vizier.nao.ac.jp",
    },
    Mirror {
        name: "China-VO Beijing",
        url: "http://vizier.china-vo.org",
    },
];

/// Julian date of the Gaia DR3 reference epoch J2016.0.
pub const EPOCH_JD: f64 = 2_457_389.0;

/// Faintest stars requested. Fainter Gaia stars are too faint to centroid
/// well on a 45 s Pan-STARRS exposure and only make the reply larger.
pub const MAX_MAG: f64 = 21.0;

#[derive(Debug, Clone, PartialEq)]
pub struct GaiaStar {
    pub source_id: u64,
    /// ICRS position at epoch J2016.0, degrees.
    pub ra: f64,
    pub dec: f64,
    /// Proper motion in mas/yr; `pm_ra` already includes cos Dec. `None`
    /// for two-parameter solutions (no proper motion measured).
    pub pm: Option<(f64, f64)>,
    /// G-band mean magnitude.
    pub g: f64,
    pub bp_rp: Option<f64>,
}

impl GaiaStar {
    /// ICRS position (degrees) at UTC Julian date `jd`, moved linearly in
    /// the tangent plane by the proper motion. Over the ~10 years since
    /// 2016 the second-order terms are below 0.1 mas for all but the very
    /// fastest stars; parallax (below a few mas for field stars) is ignored.
    pub fn position_at(&self, jd: f64) -> (f64, f64) {
        let Some((pm_ra, pm_dec)) = self.pm else {
            return (self.ra, self.dec);
        };
        let years = (jd - EPOCH_JD) / 365.25;
        let mas = 1.0 / 3_600_000.0;
        let dec = self.dec + pm_dec * years * mas;
        let cos = self.dec.to_radians().cos().max(1e-9);
        let ra = self.ra + pm_ra * years * mas / cos;
        (ra.rem_euclid(360.0), dec)
    }
}

/// ASU-TSV cone search on `mirror` for Gaia DR3 stars within `radius`
/// degrees of (`ra`, `dec`), brighter than [`MAX_MAG`].
pub fn query_url(mirror: &Mirror, ra: f64, dec: f64, radius: f64) -> String {
    let arcsec = (radius * 3600.0).ceil();
    format!(
        "{}/viz-bin/asu-tsv?-source=I/355/gaiadr3&-c={ra:.5}%20{dec:+.5}&-c.rs={arcsec:.0}\
         &-out=Source,RA_ICRS,DE_ICRS,pmRA,pmDE,Gmag,BP-RP&Gmag=%3C{MAX_MAG:.1}\
         &-out.max=unlimited&-sort=Gmag",
        mirror.url
    )
}

/// Parses VizieR's tab-separated reply. Errors when the reply is anything
/// but a Gaia table with at least one row: an HTML page, an `#INFO
/// Error=` or `no connection` report, or an empty table.
pub fn parse_tsv(text: &str) -> Result<Vec<GaiaStar>, String> {
    let head = text.trim_start();
    if head.starts_with('<') || head[..head.len().min(512)].to_lowercase().contains("<html") {
        return Err("the server sent a web page instead of the catalogue".into());
    }
    for line in text.lines().filter(|l| l.starts_with('#')) {
        if let Some((_, message)) = line.split_once("Error=") {
            return Err(format!("the server reported: {}", message.trim()));
        }
        if line.contains("no connection") || line.contains("Couldn't connect") {
            return Err("the server could not reach its database".into());
        }
    }

    let mut lines = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty());
    let header: Vec<&str> = lines
        .next()
        .ok_or("the reply has no Gaia stars")?
        .split('\t')
        .map(str::trim)
        .collect();
    let column = |name: &str| header.iter().position(|h| *h == name);
    let (Some(c_id), Some(c_ra), Some(c_dec), Some(c_g)) = (
        column("Source"),
        column("RA_ICRS"),
        column("DE_ICRS"),
        column("Gmag"),
    ) else {
        return Err("the reply is not a Gaia DR3 table".into());
    };
    let (c_pmra, c_pmde, c_color) = (column("pmRA"), column("pmDE"), column("BP-RP"));

    let mut stars = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
        // The units line and the dashes under the header.
        if fields
            .first()
            .is_some_and(|f| f.is_empty() || f.starts_with('-'))
        {
            continue;
        }
        let num = |c: Option<usize>| -> Option<f64> {
            fields
                .get(c?)?
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
        };
        let (Some(source_id), Some(ra), Some(dec), Some(g)) = (
            fields.get(c_id).and_then(|f| f.parse::<u64>().ok()),
            num(Some(c_ra)),
            num(Some(c_dec)),
            num(Some(c_g)),
        ) else {
            continue;
        };
        if !(0.0..360.0).contains(&ra) || !(-90.0..=90.0).contains(&dec) {
            continue;
        }
        let pm = num(c_pmra).zip(num(c_pmde));
        stars.push(GaiaStar {
            source_id,
            ra,
            dec,
            pm,
            g,
            bp_rp: num(c_color),
        });
    }
    if stars.is_empty() {
        return Err("the reply has no Gaia stars".into());
    }
    Ok(stars)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply in the documented ASU-TSV layout (comment block, header,
    /// units, dashes, rows), with made-up stars.
    const GOOD: &str = "#
#   VizieR Astronomical Server vizier.cfa.harvard.edu
#    Date: 2026-11-17T12:59:49 [V7.6.1]
#
#Coosys\tJ2000:\teq_FK5 J2000
#INFO\tservice_protocol=ASU\t  IVOID of the protocol through which the data was retrieved
#INFO\t-c=021.280000+04.130000,rs=60.\t

#RESOURCE=yCat_1355
#Name: I/355
#Coosys\tH_2016.000:\tICRS
#Table\tI_355_gaiadr3:
#Column\tSource\t(I19)\tUnique source identifier (source_id)
#Column\tRA_ICRS\t(F15.11)\tRight ascension (ICRS) at Ep=2016.0 (ra)
#Column\tDE_ICRS\t(F15.11)\tDeclination (ICRS) at Ep=2016.0 (dec)
#Column\tpmRA\t(F9.3)\t? Proper motion in right ascension direction, pmRA*cosDE (pmra)
#Column\tpmDE\t(F9.3)\t? Proper motion in declination direction (pmdec)
#Column\tGmag\t(F9.6)\t? G-band mean magnitude (phot_g_mean_mag)
#Column\tBP-RP\t(F9.6)\t? BP-RP colour (bp_rp)
Source\tRA_ICRS\tDE_ICRS\tpmRA\tpmDE\tGmag\tBP-RP
 \tdeg\tdeg\tmas/yr\tmas/yr\tmag\tmag
-------------------\t---------------\t---------------\t---------\t---------\t---------\t---------
1000000000000000001\t021.27564403009\t+04.11976079309\t   22.231\t    1.731\t20.257696\t 2.212534
1000000000000000002\t021.27541960943\t-04.12558594803\t   55.335\t   -5.979\t16.046635\t 2.652694
1000000000000000003\t359.99999000000\t+89.10000000000\t         \t         \t19.500000\t

";

    #[test]
    fn parses_documented_layout() {
        let stars = parse_tsv(GOOD).unwrap();
        assert_eq!(stars.len(), 3);
        let s = &stars[1];
        assert_eq!(s.source_id, 1_000_000_000_000_000_002);
        assert!((s.ra - 21.27541960943).abs() < 1e-12);
        assert!((s.dec + 4.12558594803).abs() < 1e-12);
        assert_eq!(s.pm, Some((55.335, -5.979)));
        assert!((s.g - 16.046635).abs() < 1e-9);
        assert_eq!(s.bp_rp, Some(2.652694));
        // Two-parameter solution: no proper motion, no colour.
        assert_eq!(stars[2].pm, None);
        assert_eq!(stars[2].bp_rp, None);
    }

    /// The three failure replies seen on 2026-11-17, abbreviated.
    #[test]
    fn rejects_mirror_failures() {
        let cds_html = "<!DOCTYPE html>\n<html lang=\"en\">\n<head><title>Page unavailable</title>\
                        </head>\n<body><p class=\"error-code\">404</p>\
                        <h1>This page is currently unavailable.</h1></body></html>\n";
        let naoj = "#\n#   VizieR Astronomical Server vizier.nao.ac.jp\n#\n\
                    #INFO\tError=The database is not currently reachable\n\n#RESOURCE=\n#Name:\n";
        let naoj_other = "#\n# -- no connection\n# Report from: stdb_pg_conn,     Status: STDB_ERR\n\
                          #Couldn't connect with server\n#\n\n#RESOURCE=\n#Name:\n#Title:\n";
        let china_vo = "#\n#   VizieR Astronomical Server vizier.china-vo.org\n\
                        #    Date: 2026-11-17T13:00:37 [V7.6.1]\n#\n\
                        #Coosys\tJ2000:\teq_FK5 J2000\n\n#RESOURCE=\n#Name:\n#Title:\n";
        assert!(parse_tsv(cds_html).unwrap_err().contains("web page"));
        assert!(
            parse_tsv(naoj)
                .unwrap_err()
                .contains("not currently reachable")
        );
        assert!(parse_tsv(naoj_other).is_err());
        assert!(parse_tsv(china_vo).unwrap_err().contains("no Gaia stars"));
        // A table with its header but no rows is no better.
        let empty: String = GOOD
            .lines()
            .take_while(|l| !l.starts_with("1000"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(parse_tsv(&empty).unwrap_err().contains("no Gaia stars"));
        assert!(parse_tsv("").is_err());
    }

    #[test]
    fn url_asks_for_the_needed_columns() {
        let url = query_url(&MIRRORS[1], 21.28, 4.13, 0.125);
        assert!(url.starts_with("https://vizier.cfa.harvard.edu/viz-bin/asu-tsv?"));
        assert!(url.contains("-source=I/355/gaiadr3"));
        assert!(url.contains("-c=21.28000%20+4.13000"));
        assert!(url.contains("-c.rs=450"));
        assert!(url.contains("-out=Source,RA_ICRS,DE_ICRS,pmRA,pmDE,Gmag,BP-RP"));
        assert!(url.contains("Gmag=%3C21.0&"), "{url}");
        assert!(url.contains("-out.max=unlimited"));
        assert!(!url.contains(' '));
    }

    #[test]
    fn proper_motion_moves_the_star() {
        let star = GaiaStar {
            source_id: 1,
            ra: 10.0,
            dec: 60.0,
            pm: Some((1000.0, -2000.0)), // mas/yr
            g: 15.0,
            bp_rp: None,
        };
        // Exactly ten Julian years later.
        let (ra, dec) = star.position_at(EPOCH_JD + 3652.5);
        assert!((dec - (60.0 - 20.0 / 3600.0)).abs() < 1e-12);
        // 10″ of great circle at Dec 60° is 20″ of RA.
        assert!(
            ((ra - 10.0) * 3600.0 - 20.0).abs() < 1e-9,
            "{}",
            (ra - 10.0) * 3600.0
        );
        let still = GaiaStar { pm: None, ..star };
        assert_eq!(still.position_at(EPOCH_JD + 3652.5), (10.0, 60.0));
    }
}
