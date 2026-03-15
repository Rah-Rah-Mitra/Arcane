//! OCR overlay tier — renders PDF pages via pdfium-render and recognises text
//! with oar-ocr (PaddleOCR v5 mobile models).
//!
//! Results are returned as `PositionedText` so the rest of the pipeline is
//! unchanged.  This module is compiled only when the `ocr` feature is enabled.
//!
//! # Model setup (one-time, per machine)
//!
//! Download the PaddleOCR v5 **English** models from oar-ocr releases and
//! place them under `models/` relative to the binary:
//!
//! | File | Default path | Env override |
//! |------|-------------|--------------|
//! | Detection | `models/pp-ocrv5_mobile_det.onnx` | `ARCANE_OCR_DET_MODEL` |
//! | Recognition (English) | `models/en_pp-ocrv5_mobile_rec.onnx` | `ARCANE_OCR_REC_MODEL` |
//! | Dictionary (English) | `models/ppocrv5_en_dict.txt` | `ARCANE_OCR_DICT` |
//!
//! For non-English PDFs, set the env vars to point at the appropriate
//! language-specific recognition model and dictionary (e.g. `pp-ocrv5_mobile_rec.onnx`
//! + `ppocrv5_dict.txt` for Chinese).
//!
//! Also requires `onnxruntime.dll` (v1.24.x) at runtime — set `ORT_DYLIB_PATH`
//! or place it next to the binary.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use oar_ocr::oarocr::OAROCRBuilder;
use pdfium_render::prelude::*;

use super::layout::PositionedText;

// ---------------------------------------------------------------------------
// ORT DLL initialisation (load-dynamic mode)
// ---------------------------------------------------------------------------

const ORT_DYLIB_ENV: &str = "ORT_DYLIB_PATH";
const ORT_DYLIB_DEFAULT: &str = "onnxruntime.dll";

// Store Result<(), String> — String is Clone so it can be stored in OnceLock.
static ORT_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Ensure ONNX Runtime is loaded from the DLL exactly once per process.
///
/// Under the `load-dynamic` feature, `ort` does not static-link ONNX Runtime;
/// instead it dlopen()s the shared library at runtime.  The path is resolved
/// from `ORT_DYLIB_PATH` (if set) or falls back to `"onnxruntime.dll"` next
/// to the binary / in PATH.
///
/// Call this before any `oar-ocr` / `ort` API use.
fn ensure_ort_loaded() -> anyhow::Result<()> {
    let result = ORT_INIT.get_or_init(|| {
        let dll = std::env::var(ORT_DYLIB_ENV).unwrap_or_else(|_| ORT_DYLIB_DEFAULT.to_string());
        ort::init_from(&dll)
            .map(|builder| {
                // commit() returns bool (true = newly set, false = already set).
                builder.commit();
            })
            .map_err(|e| {
                format!(
                    "Cannot load ONNX Runtime DLL from '{dll}'.\n\
                 {e}\n\
                 Download onnxruntime-win-x64-1.24.1.zip from\n\
                 https://github.com/microsoft/onnxruntime/releases/tag/v1.24.1\n\
                 Place onnxruntime.dll next to the arcane binary,\n\
                 or set {ORT_DYLIB_ENV}=<path/to/onnxruntime.dll>"
                )
            })
    });
    // OnceLock stores the first result; re-raise the error string if it failed.
    match result {
        Ok(()) => Ok(()),
        Err(e) => anyhow::bail!("{e}"),
    }
}

// ---------------------------------------------------------------------------
// Model path constants
// ---------------------------------------------------------------------------

const DET_MODEL_ENV: &str = "ARCANE_OCR_DET_MODEL";
const REC_MODEL_ENV: &str = "ARCANE_OCR_REC_MODEL";
const DICT_ENV: &str = "ARCANE_OCR_DICT";

const DET_DEFAULT: &str = "models/pp-ocrv5_mobile_det.onnx";
const REC_DEFAULT: &str = "models/en_pp-ocrv5_mobile_rec.onnx";
const DICT_DEFAULT: &str = "models/ppocrv5_en_dict.txt";

fn model_path(env: &str, default: &str) -> String {
    std::env::var(env).unwrap_or_else(|_| default.to_string())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render the given 0-based `page_indices` of the PDF at `dpi`, run OAR-OCR,
/// and return heading-sized text regions as `PositionedText`.
///
/// Only regions whose bounding-box height is ≥ 1.5% of the page height are
/// returned — this filters body text and focuses on heading-scale runs.
///
/// # Errors
///
/// Returns an error if the OCR models cannot be loaded, pdfium fails to
/// open the document, or any page fails to render.
pub fn extract_headings_ocr(
    path: &Path,
    page_indices: &[u32],
    dpi: u32,
) -> Result<Vec<PositionedText>> {
    if page_indices.is_empty() {
        return Ok(Vec::new());
    }

    // Load ONNX Runtime DLL (no-op after the first call in this process).
    ensure_ort_loaded()?;

    // --- Load OCR engine ---------------------------------------------------
    let det = model_path(DET_MODEL_ENV, DET_DEFAULT);
    let rec = model_path(REC_MODEL_ENV, REC_DEFAULT);
    let dict = model_path(DICT_ENV, DICT_DEFAULT);

    let ocr = OAROCRBuilder::new(&det, &rec, &dict)
        .build()
        .with_context(|| {
            format!(
                "Failed to load OCR models.\n\
             Expected:\n  {det}\n  {rec}\n  {dict}\n\
             Download from https://github.com/GreatV/oar-ocr/releases\n\
             Or set env vars: {DET_MODEL_ENV}, {REC_MODEL_ENV}, {DICT_ENV}"
            )
        })?;

    // --- Load PDF via pdfium -----------------------------------------------
    let pdfium = Pdfium::new(
        Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("."))
            .or_else(|_| Pdfium::bind_to_system_library())
            .context("Failed to bind pdfium library")?,
    );
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .context("pdfium: failed to open PDF")?;

    let scale = dpi as f32 / 72.0;

    let mut results: Vec<PositionedText> = Vec::new();

    for &page_idx in page_indices {
        let page = doc
            .pages()
            .get(page_idx as u16)
            .with_context(|| format!("pdfium: page {page_idx} out of range"))?;

        let page_h_pts = page.height().value;

        // Render page to a DynamicImage at the requested DPI.
        let render_config = PdfRenderConfig::new().scale_page_by_factor(scale);
        let dynamic_img = page
            .render_with_config(&render_config)
            .context("pdfium: page render failed")?
            .as_image();

        let img_h_px = dynamic_img.height() as f32;
        // Only keep regions whose pixel height is ≥ 1.5% of the page height —
        // this approximates heading-sized text, skipping body paragraphs.
        let min_box_h_px = img_h_px * 0.015;

        // oar-ocr's predict() expects RgbImage (ImageBuffer<Rgb<u8>, Vec<u8>>).
        let rgb_img = dynamic_img.to_rgb8();

        // Run OAR-OCR on the rendered image.
        let ocr_output = ocr
            .predict(vec![rgb_img])
            .context("OCR prediction failed")?;

        let regions = &ocr_output[0].text_regions;
        for region in regions {
            let Some((text, confidence)) = region.text_with_confidence() else {
                continue;
            };
            if confidence < 0.6 {
                continue;
            }

            // `bounding_box` is a field; use x_min/y_min/x_max/y_max accessors.
            let bbox = &region.bounding_box;
            let box_h_px = bbox.y_max() - bbox.y_min();
            if box_h_px < min_box_h_px {
                continue; // body-text sized — skip
            }

            // Convert pixel coords (top-left origin) → PDF points (bottom-left origin).
            let x_pt = bbox.x_min() / scale;
            // PDF Y grows upward from bottom; image Y grows downward from top.
            let y_pt = page_h_pts - (bbox.y_max() / scale);
            let font_size_pt = box_h_px / scale;

            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }

            results.push(PositionedText {
                page_index: page_idx,
                x: x_pt,
                y: y_pt,
                font_size: font_size_pt,
                font_key: "OCR".into(),
                text: trimmed,
            });
        }
    }

    Ok(results)
}
