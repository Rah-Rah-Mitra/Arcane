//! OCR overlay tier — renders PDF pages via pdfium-render and recognises text
//! with oar-ocr (PaddleOCR v5 mobile models).
//!
//! Results are returned as `PositionedText` so the rest of the pipeline is
//! unchanged.  This module is compiled only when the `ocr` feature is enabled.
//!
//! # Model setup
//!
//! Run `arcane init-ocr` to download all required models and runtime
//! libraries to `~/Arcane/models/`.  Models are auto-detected at runtime.
//!
//! To override paths, set these environment variables:
//!
//! | Env var | Purpose |
//! |---------|---------|
//! | `ARCANE_OCR_DET_MODEL` | Detection ONNX model |
//! | `ARCANE_OCR_REC_MODEL` | Recognition ONNX model |
//! | `ARCANE_OCR_DICT` | Recognition dictionary |
//! | `ORT_DYLIB_PATH` | ONNX Runtime shared library |

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use oar_ocr::oarocr::{OAROCRBuilder, OAROCR};
use pdfium_render::prelude::*;

use super::layout::PositionedText;

// ---------------------------------------------------------------------------
// ORT DLL initialisation (load-dynamic mode)
// ---------------------------------------------------------------------------

const ORT_DYLIB_ENV: &str = "ORT_DYLIB_PATH";

#[cfg(target_os = "windows")]
const ORT_DYLIB_DEFAULT: &str = "onnxruntime.dll";
#[cfg(target_os = "linux")]
const ORT_DYLIB_DEFAULT: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
const ORT_DYLIB_DEFAULT: &str = "libonnxruntime.dylib";
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
const ORT_DYLIB_DEFAULT: &str = "libonnxruntime.so";

static ORT_INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Ensure ONNX Runtime is loaded from the DLL exactly once per process.
fn ensure_ort_loaded() -> anyhow::Result<()> {
    let result = ORT_INIT.get_or_init(|| {
        let dll = std::env::var(ORT_DYLIB_ENV).unwrap_or_else(|_| {
            // Check ~/Arcane/models/ before falling back to system search.
            if let Ok(dir) = crate::storage::filesystem::models_dir() {
                let candidate = dir.join(ORT_DYLIB_DEFAULT);
                if candidate.exists() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
            ORT_DYLIB_DEFAULT.to_string()
        });
        ort::init_from(&dll)
            .map(|builder| {
                builder.commit();
            })
            .map_err(|e| format!(
                "Cannot load ONNX Runtime from '{dll}'.\n\
                 {e}\n\
                 Run `arcane init-ocr` to download it,\n\
                 or set {ORT_DYLIB_ENV}=<path/to/library>"
            ))
    });
    match result {
        Ok(()) => Ok(()),
        Err(e) => anyhow::bail!("{e}"),
    }
}

// ---------------------------------------------------------------------------
// Model path resolution
// ---------------------------------------------------------------------------

const DET_MODEL_ENV: &str = "ARCANE_OCR_DET_MODEL";
const REC_MODEL_ENV: &str = "ARCANE_OCR_REC_MODEL";
const DICT_ENV: &str = "ARCANE_OCR_DICT";

const DET_FILENAME: &str = "pp-ocrv5_mobile_det.onnx";
const REC_FILENAME: &str = "en_pp-ocrv5_mobile_rec.onnx";
const DICT_FILENAME: &str = "ppocrv5_en_dict.txt";

/// Resolve a model path: env var → ~/Arcane/models/ → CWD-relative fallback.
fn model_path(env: &str, filename: &str) -> String {
    if let Ok(val) = std::env::var(env) {
        return val;
    }
    if let Ok(dir) = crate::storage::filesystem::models_dir() {
        let candidate = dir.join(filename);
        if candidate.exists() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    format!("models/{filename}")
}

// ---------------------------------------------------------------------------
// OCR engine cache (OnceLock — OAROCR is Send + Sync)
// ---------------------------------------------------------------------------

static OCR_ENGINE: OnceLock<std::result::Result<OAROCR, String>> = OnceLock::new();

/// Get or build the cached OCR engine. The engine is initialised once per
/// process and reused across all subsequent calls.
fn get_ocr_engine() -> anyhow::Result<&'static OAROCR> {
    let result = OCR_ENGINE.get_or_init(|| {
        let det = model_path(DET_MODEL_ENV, DET_FILENAME);
        let rec = model_path(REC_MODEL_ENV, REC_FILENAME);
        let dict = model_path(DICT_ENV, DICT_FILENAME);
        OAROCRBuilder::new(&det, &rec, &dict)
            .build()
            .map_err(|e| format!(
                "Failed to load OCR models.\n  {det}\n  {rec}\n  {dict}\n\
                 Run `arcane init-ocr` to download them.\nError: {e}"
            ))
    });
    match result {
        Ok(engine) => Ok(engine),
        Err(e) => anyhow::bail!("{e}"),
    }
}

// ---------------------------------------------------------------------------
// Pdfium binding — check ~/Arcane/models/ first
// ---------------------------------------------------------------------------

fn bind_pdfium() -> Result<Pdfium> {
    let try_path =
        |dir: &str| Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(dir));

    let arcane_models = crate::storage::filesystem::models_dir().ok();
    let bindings = arcane_models
        .as_ref()
        .and_then(|p| try_path(&p.to_string_lossy()).ok())
        .or_else(|| try_path(".").ok())
        .or_else(|| Pdfium::bind_to_system_library().ok())
        .context("Failed to bind pdfium. Run `arcane init-ocr` to download it.")?;
    Ok(Pdfium::new(bindings))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Maximum images per OCR batch to limit memory usage.
const BATCH_SIZE: usize = 16;

/// Render the given 0-based `page_indices` of the PDF at `dpi`, run OAR-OCR,
/// and return heading-sized text regions as `PositionedText`.
///
/// Only regions whose bounding-box height is >= 1.5% of the page height are
/// returned — this filters body text and focuses on heading-scale runs.
///
/// The OCR engine is cached across calls (OnceLock) and pages are batched
/// (up to 16 at a time) to minimise ONNX session overhead.
pub fn extract_headings_ocr(
    path: &Path,
    page_indices: &[u32],
    dpi: u32,
) -> Result<Vec<PositionedText>> {
    if page_indices.is_empty() {
        return Ok(Vec::new());
    }

    // Load ONNX Runtime DLL (no-op after the first call).
    ensure_ort_loaded()?;

    // Get cached OCR engine (built once per process).
    let ocr = get_ocr_engine()?;

    // Load PDF via pdfium (searches ~/Arcane/models/, CWD, then system).
    let pdfium = bind_pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .context("pdfium: failed to open PDF")?;

    let scale = dpi as f32 / 72.0;

    // Phase 1: render all pages to images.
    // Use DynamicImage::to_rgb8() which returns ImageBuffer<Rgb<u8>, Vec<u8>> —
    // the exact type oar-ocr's predict() expects.
    let mut images = Vec::with_capacity(page_indices.len());
    let mut meta: Vec<(u32, f32, f32)> = Vec::with_capacity(page_indices.len());

    for &page_idx in page_indices {
        let page = doc
            .pages()
            .get(page_idx as u16)
            .with_context(|| format!("pdfium: page {page_idx} out of range"))?;

        let page_h_pts = page.height().value;
        let render_config = PdfRenderConfig::new().scale_page_by_factor(scale);
        let dynamic_img = page
            .render_with_config(&render_config)
            .context("pdfium: page render failed")?
            .as_image();

        let img_h_px = dynamic_img.height() as f32;
        images.push(dynamic_img.to_rgb8());
        meta.push((page_idx, page_h_pts, img_h_px));
    }

    // Phase 2: batch OCR — process in chunks of BATCH_SIZE.
    let mut results: Vec<PositionedText> = Vec::new();
    let mut img_idx = 0;

    while !images.is_empty() {
        let batch_size = images.len().min(BATCH_SIZE);
        let batch: Vec<_> = images.drain(..batch_size).collect();
        let batch_meta = &meta[img_idx..img_idx + batch_size];
        img_idx += batch_size;

        let outputs = ocr
            .predict(batch)
            .context("OCR batch predict failed")?;

        for (i, output) in outputs.iter().enumerate() {
            let (page_idx, page_h_pts, img_h_px) = batch_meta[i];
            let min_box_h_px = img_h_px * 0.015;

            for region in &output.text_regions {
                let Some((text, confidence)) = region.text_with_confidence() else {
                    continue;
                };
                if confidence < 0.6 {
                    continue;
                }

                let bbox = &region.bounding_box;
                let box_h_px = bbox.y_max() - bbox.y_min();
                if box_h_px < min_box_h_px {
                    continue;
                }

                // Convert pixel coords (top-left origin) -> PDF points (bottom-left origin).
                let x_pt = bbox.x_min() / scale;
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
    }

    Ok(results)
}
