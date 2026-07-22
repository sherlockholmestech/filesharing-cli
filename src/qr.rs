use anyhow::{Context, Result};
use image::Luma;
use qrcode::{QrCode, render::unicode::Dense1x2};
use std::path::{Path, PathBuf};

pub fn generate(url: &str, file_path: &Path) -> Result<PathBuf> {
    let code = QrCode::new(url.as_bytes()).context("Could not encode upload URL as QR code")?;
    let terminal = code
        .render::<Dense1x2>()
        .quiet_zone(true)
        .module_dimensions(1, 1)
        .build();
    println!("\n{terminal}");

    let output = output_path(file_path)?;
    if output.exists() {
        anyhow::bail!("QR output already exists: {}", output.display());
    }
    let image = code
        .render::<Luma<u8>>()
        .quiet_zone(true)
        .min_dimensions(512, 512)
        .build();
    image
        .save(&output)
        .with_context(|| format!("Could not save QR code to {}", output.display()))?;

    Ok(output)
}

fn output_path(file_path: &Path) -> Result<PathBuf> {
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("File has no valid name")?;
    Ok(file_path.with_file_name(format!("qr_{file_name}.png")))
}

#[cfg(test)]
mod tests {
    use super::output_path;
    use std::path::Path;

    #[test]
    fn prefixes_original_filename_for_png() {
        assert_eq!(
            output_path(Path::new("/tmp/archive.tar.zst")).unwrap(),
            Path::new("/tmp/qr_archive.tar.zst.png")
        );
    }
}
