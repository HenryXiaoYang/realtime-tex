//! `--pic-cache`: a second run that takes its pictures from the cache must give the same PDF.

use super::report::*;
use anyhow::Result;
use rtex_core::texlive::TexLive;
use rtex_verify::raster;
use std::path::Path;

/// Picture cache check: build a cache from the converged capture (`cap`), run one more capture
/// pass with its manifest, and compare units, placements and (optionally) rendered pages.
pub(super) fn pic_cache_check(
    tl: &TexLive,
    project: &Path,
    opts: &VerifyOpts,
    unit_envs: &str,
    cap: &rtex_core::capture::CaptureResult,
) -> Result<PicCacheReport> {
    use rtex_core::piccache::{scan_pictures, PicCache};
    let mut rep = PicCacheReport::default();
    let files = rtex_core::document::load_project_files(project, &opts.main)?;
    let pics = scan_pictures(
        &files,
        &opts.main,
        rtex_core::piccache::preamble_hash(&files, &opts.main),
    );
    rep.pictures = pics.len();
    rep.cacheable = pics.iter().filter(|p| p.cacheable).count();
    let recorded = cap.json.recorded_pics();
    rep.recorded = pics
        .iter()
        .filter(|p| p.cacheable && recorded.contains_key(&p.key))
        .count();
    rep.wall_uncached_ms = cap.wall.as_millis() as u64;
    if rep.cacheable == 0 {
        println!("picture cache: no cacheable picture environment in the sources");
        rep.units_equal = true;
        rep.placements_equal = true;
        rep.pass = true;
        return Ok(rep);
    }
    // the second pass runs on the converged aux family (same labels, TOC and bibliography)
    let out = opts.build.join("piccache");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    rtex_core::background::copy_aux_family(&cap.out_dir, &out)?;
    let mut cache = PicCache::open(&out.join("pic-cache"));
    cache.absorb(&pics, &recorded, &[], &cap.pdf)?;
    rep.hits = cache.write_manifest(&pics, &out.join("pic-manifest.json"))?;
    let cached =
        rtex_core::capture::run_capture_with(tl, project, &opts.main, &out, true, unit_envs, None)?;
    rep.wall_cached_ms = cached.wall.as_millis() as u64;
    rep.mismatched = cached.json.pic_mismatch.len();
    // units and placements
    let (a, b) = (&cap.json.units, &cached.json.units);
    rep.units_equal = a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.kind == y.kind
                && x.name == y.name
                && x.begin_line == y.begin_line
                && x.end_line == y.end_line
        });
    let mut differing = Vec::new();
    for (x, y) in a.iter().zip(b) {
        let same = x.placements.len() == y.placements.len()
            && x.placements.iter().zip(&y.placements).all(|(p, q)| {
                (p.page, p.x, p.y, p.w, p.h, p.d) == (q.page, q.x, q.y, q.w, q.h, q.d)
            });
        if !same {
            differing.push(x.uid);
        }
    }
    rep.placements_equal = rep.units_equal && differing.is_empty();
    for n in 1..=cached.json.pages {
        if let Ok(dl) = cached.page(n) {
            rep.cached_images += dl
                .flags_map()
                .get("pic_cache")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
        }
    }
    // rendered pages of the two passes
    if opts.raster {
        let mut worst = 0.0f64;
        for n in 1..=cap.json.pages.min(cached.json.pages) {
            let pa = out.join(format!("uncached-p{n}.png"));
            let pb = out.join(format!("cached-p{n}.png"));
            raster::render_pdf_page_with_pymupdf(&cap.pdf, n as u32, opts.dpi, &pa)?;
            raster::render_pdf_page_with_pymupdf(&cached.pdf, n as u32, opts.dpi, &pb)?;
            let (ga, wa, ha) = raster::read_png_gray(&pa)?;
            let (gb, wb, hb) = raster::read_png_gray(&pb)?;
            if (wa, ha) != (wb, hb) {
                worst = 1.0;
                println!("  picture cache page {n}: raster size mismatch");
                continue;
            }
            let diff = raster::compare(&ga, &gb, wa, ha, 96);
            if diff.unmatched_fraction > 0.001 {
                println!(
                    "  picture cache page {n}: unmatched ink {:.3}% (bbox {:?})",
                    diff.unmatched_fraction * 100.0,
                    diff.bbox_unmatched
                );
            }
            worst = worst.max(diff.unmatched_fraction);
        }
        rep.raster_unmatched_fraction = Some(worst);
    }
    // the check is meaningless when nothing was cached: a cacheable document must record
    // and reuse pictures (at least `min_cached` of them when the caller says how many)
    let enough = rep.recorded > 0 && rep.recorded >= opts.min_cached.unwrap_or(1);
    rep.pass = rep.units_equal
        && rep.placements_equal
        && cap.json.pages == cached.json.pages
        && enough
        && rep.hits == rep.recorded
        && rep.cached_images == rep.hits
        && cached.json.pic_mismatch.is_empty()
        && rep.raster_unmatched_fraction.unwrap_or(0.0) <= 0.001;
    println!(
        "picture cache: {} pictures, {} cacheable, {} recorded by the first pass, {} taken from it by the second ({} cached images on its pages, {} mismatched); units equal {}, placements equal {}{}; pass {:.1}s -> {:.1}s; pass={}",
        rep.pictures,
        rep.cacheable,
        rep.recorded,
        rep.hits,
        rep.cached_images,
        rep.mismatched,
        rep.units_equal,
        rep.placements_equal,
        match rep.raster_unmatched_fraction {
            Some(f) => format!(", worst unmatched ink {:.3}%", f * 100.0),
            None => String::new(),
        },
        rep.wall_uncached_ms as f64 / 1000.0,
        rep.wall_cached_ms as f64 / 1000.0,
        rep.pass
    );
    if !differing.is_empty() {
        println!(
            "  units with different placements: {:?}",
            &differing[..differing.len().min(10)]
        );
    }
    Ok(rep)
}
