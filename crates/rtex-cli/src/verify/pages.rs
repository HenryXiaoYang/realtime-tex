//! Layers 2 and 3: every page's display list against the clean PDF's content stream, and, with `--raster`, its rendering against the PDF's.

use super::report::*;
use anyhow::Result;
use rtex_dl::{DisplayList, Item};
use rtex_verify::compare::compare_page;
use rtex_verify::{pdftext, raster};

/// Layers 2 and 3, page by page; fills the report's `pages` and prints the layers' summaries.
pub(super) fn check_pages(
    cap: &rtex_core::capture::CaptureResult,
    clean: &rtex_core::capture::CaptureResult,
    opts: &VerifyOpts,
    report: &mut Report,
) -> Result<()> {
    let pdf_pages = pdftext::extract(&clean.pdf)?;
    let mut cache = raster::FontCache::default();
    let mut l2_ok = true;
    let mut l3_ok: Option<bool> = if opts.raster { Some(true) } else { None };
    for pdf_page in &pdf_pages {
        let page_no = pdf_page.number as i64;
        let dl = cap.page(page_no)?;
        let dec_quantum = 10f64.powi(-(pdf_page.decimals.max(1) as i32));
        let rep = compare_page(&dl, pdf_page, &[], dec_quantum);
        // TJ quantum for the largest text size on the page
        let max_size = pdf_page.glyphs.iter().map(|g| g.size).fold(0.0, f64::max);
        let tj_quantum = max_size / 1000.0;
        let interior_max = rep.max_dx_bp;
        let colors_dl = dl
            .lines
            .iter()
            .flat_map(|l| l.items.iter())
            .chain(dl.other.iter())
            .filter(|i| matches!(i, Item::Color { .. }))
            .count();
        let dl_images = image_rectangles(&dl, pdf_page);
        let images_matched = dl_images
            .iter()
            .filter(|(x, y, w, h)| {
                pdf_page.images.iter().any(|im| {
                    (im.x - x).abs() < 0.01
                        && (im.y - y).abs() < 0.01
                        && (im.w - w).abs() < 0.01
                        && (im.h - h).abs() < 0.01
                })
            })
            .count();
        let mut pv = PageVerdict {
            page: page_no,
            dl_glyphs: rep.dl_glyphs,
            pdf_glyphs: rep.pdf_glyphs,
            matched: rep.matched,
            unmatched: rep.unmatched,
            anchored_max_dx_bp: rep.anchored_max_dx_bp,
            interior_max_dx_bp: interior_max,
            max_dy_bp: rep.max_dy_bp,
            tj_quantum_bp: tj_quantum,
            rules_dl: rep.rules_dl,
            rules_matched: rep.rules_matched,
            colors_dl,
            colors_pdf: pdf_page.color_ops.len(),
            exact: dl.is_exact(),
            flags: dl.flags.clone(),
            images_dl: dl_images.len(),
            images_matched,
            images_pdf: pdf_page.images.len(),
            ..Default::default()
        };
        let page_ok = rep.unmatched == 0
            && rep.dl_glyphs == rep.pdf_glyphs
            && rep.anchored_max_dx_bp <= dec_quantum * 1.5
            && interior_max <= tj_quantum * 1.05
            && rep.max_dy_bp <= dec_quantum * 1.5
            && rep.rules_matched == rep.rules_dl
            && images_matched == dl_images.len()
            && dl_images.len() == pdf_page.images.len();
        let degraded = !dl.is_exact();
        if !page_ok && !degraded {
            l2_ok = false;
        }
        if degraded {
            report.pages_degraded += 1;
            println!("  page {page_no}: DEGRADED {} (host falls back to the PDF page); glyphs matched {}/{} (pdf {})", dl.flags, rep.matched, rep.dl_glyphs, rep.pdf_glyphs);
        }
        if opts.raster {
            let ref_png = opts.build.join(format!("ref-p{page_no}.png"));
            raster::render_pdf_page_with_pymupdf(&clean.pdf, pdf_page.number, opts.dpi, &ref_png)?;
            let (ref_gray, rw, rh) = raster::read_png_gray(&ref_png)?;
            let (dl_gray, w, h, stats) = raster::rasterize(
                &[&dl],
                pdf_page.width,
                pdf_page.height,
                opts.dpi as f64,
                &mut cache,
            )?;
            raster::write_png(
                &opts.build.join(format!("dl-p{page_no}.png")),
                &dl_gray,
                w,
                h,
            )?;
            if (rw, rh) == (w, h) {
                let diff = raster::compare(&dl_gray, &ref_gray, w, h, 96);
                pv.raster_unmatched_fraction = Some(diff.unmatched_fraction);
                pv.raster_glyphs_skipped = Some(stats.glyphs_skipped);
                if (diff.unmatched_fraction > 0.01 || stats.glyphs_skipped > 0) && !degraded {
                    l3_ok = Some(false);
                    println!("  page {page_no} raster: unmatched {:.3}% (A {} B {} unmatched {}+{}), skipped glyphs {} {:?}, bbox {:?}", diff.unmatched_fraction * 100.0, diff.ink_a, diff.ink_b, diff.unmatched_a, diff.unmatched_b, stats.glyphs_skipped, stats.skipped_fonts, diff.bbox_unmatched);
                }
            } else {
                l3_ok = Some(false);
                println!("  page {page_no} raster size mismatch {w}x{h} vs {rw}x{rh}");
            }
        }
        if !page_ok && !degraded {
            println!("  page {page_no}: matched {}/{} (pdf {}), anchored max dx {:.5}, interior max dx {:.5} (tj {:.5}), dy {:.5}, rules {}/{}, images {}/{} (pdf {}), flags {}",
                rep.matched, rep.dl_glyphs, rep.pdf_glyphs, rep.anchored_max_dx_bp, interior_max, tj_quantum, rep.max_dy_bp, rep.rules_matched, rep.rules_dl, images_matched, dl_images.len(), pdf_page.images.len(), dl.flags);
            for w in rep.worst.iter().take(2) {
                println!("    worst: par {} line {} idx {:?} font {:?} dl=({:.4},{:.4}) pdf=({:.4},{:.4}) dx={:.5}", w.par, w.line, w.index, w.pdf_font, w.dl_x_bp, w.dl_y_bp, w.pdf_x_bp.unwrap_or(0.0), w.pdf_y_bp.unwrap_or(0.0), w.dx_bp.unwrap_or(f64::NAN));
            }
        }
        report.pages.push(pv);
    }
    report.layer2_pass = l2_ok;
    report.layer3_pass = l3_ok;
    let pages_exact = report.pages.iter().filter(|p| p.exact).count();
    println!("layer 2 (pages vs PDF content stream): {} pages, {} exact, {} degraded (PDF fallback), pass={}; worst anchored dx {:.5} bp, worst interior dx {:.5} bp",
        report.pages.len(), pages_exact, report.pages_degraded, l2_ok,
        report.pages.iter().map(|p| p.anchored_max_dx_bp).fold(0.0, f64::max), report.pages.iter().map(|p| p.interior_max_dx_bp).fold(0.0, f64::max));
    if opts.raster {
        let worst = report
            .pages
            .iter()
            .filter_map(|p| p.raster_unmatched_fraction)
            .fold(0.0, f64::max);
        println!(
            "layer 3 (rendered, {} dpi): pass={:?}; worst unmatched ink fraction {:.3}%",
            opts.dpi,
            l3_ok,
            worst * 100.0
        );
    }
    Ok(())
}

/// image rectangles with the PDF transformation state applied: graphicx scales (and
/// rotates) bitmap images with save / setmatrix / restore around the image. The affine map
/// p' = M p + t is tracked in TeX coordinates (a pure scale about a point has the same
/// form in both orientations; rotations are approximated by their bounding box).
fn image_rectangles(
    dl: &DisplayList,
    pdf_page: &rtex_verify::pdftext::PdfPage,
) -> Vec<(f64, f64, f64, f64)> {
    let mut out = Vec::new();
    let identity = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut stack: Vec<[f64; 6]> = Vec::new();
    let mut cur = identity;
    let items = dl
        .other
        .iter()
        .chain(dl.lines.iter().flat_map(|l| l.items.iter()));
    for it in items {
        match it {
            Item::Matrix { op, x, y, data } => match op.as_str() {
                "save" => stack.push(cur),
                "restore" => cur = stack.pop().unwrap_or(identity),
                _ => {
                    let v: Vec<f64> = data
                        .split_whitespace()
                        .filter_map(|t| t.parse().ok())
                        .collect();
                    if v.len() == 4 {
                        let (a2, b2, c2, d2) = (v[0], v[1], v[2], v[3]);
                        let (px, py) = (*x as f64, *y as f64);
                        // T2(p) = M2 p + (P - M2 P); T = T1 ∘ T2
                        let [a1, b1, c1, d1, tx1, ty1] = cur;
                        let t2x = px - (a2 * px + c2 * py);
                        let t2y = py - (b2 * px + d2 * py);
                        cur = [
                            a1 * a2 + c1 * b2,
                            b1 * a2 + d1 * b2,
                            a1 * c2 + c1 * d2,
                            b1 * c2 + d1 * d2,
                            a1 * t2x + c1 * t2y + tx1,
                            b1 * t2x + d1 * t2y + ty1,
                        ];
                    }
                }
            },
            Item::Image {
                x,
                y_top,
                width,
                height,
                ..
            } => {
                let [a, b, c, d, tx, ty] = cur;
                let tr = |px: f64, py: f64| (a * px + c * py + tx, b * px + d * py + ty);
                let (x0, y0) = (*x as f64, *y_top as f64);
                let (x1, y1) = (x0 + *width as f64, y0 + *height as f64);
                let pts = [tr(x0, y0), tr(x1, y0), tr(x0, y1), tr(x1, y1)];
                let minx = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
                let maxx = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
                let miny = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                let maxy = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                out.push((
                    minx / rtex_dl::SP_PER_BP,
                    pdf_page.height - maxy / rtex_dl::SP_PER_BP,
                    (maxx - minx) / rtex_dl::SP_PER_BP,
                    (maxy - miny) / rtex_dl::SP_PER_BP,
                ));
            }
            _ => {}
        }
    }
    out
}
