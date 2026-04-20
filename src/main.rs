use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{env, path::PathBuf};

mod download;
mod http;
mod progress;
mod providers;
mod settings;
mod token;

use providers::{
    fichier::OneFichier, fuckingfast::FuckingFast, gofile::Gofile, pixeldrain::Pixeldrain,
    rootz::Rootz, vikingfile::VikingFile,
};

#[derive(Parser)]
#[command(
    name = "fsc",
    about = "Upload files to temporary file sharing platforms",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a file from a URL or provider share link
    Download {
        /// URL or share link to download (supports 1fichier, gofile, pixeldrain, rootz; others: use direct URL)
        url: String,

        /// Output file path (default: derived from URL or Content-Disposition header)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// API token (prefer FSC_TOKEN or provider-specific FSC_*_TOKEN env vars)
        #[arg(short, long)]
        token: Option<String>,
    },

    /// Upload a file
    Upload {
        /// Path to the file to upload
        file: PathBuf,

        /// Provider: gofile (gf), fuckingfast (ff), pixeldrain (pd), vikingfile (vf), rootz (rz), 1fichier (1f)
        #[arg(short, long, default_value = "gofile")]
        provider: String,

        /// API token — gofile: API token · fuckingfast: account ID ·
        /// pixeldrain: API key · vikingfile: user hash · rootz: API key ·
        /// 1fichier: API key (prefer FSC_TOKEN or provider-specific FSC_*_TOKEN env vars)
        #[arg(short, long)]
        token: Option<String>,

        /// Attach a note (fuckingfast only)
        #[arg(short, long)]
        note: Option<String>,

        /// Folder — gofile/rootz: folder ID · vikingfile: path e.g. "Videos/Clips" ·
        /// fuckingfast: parent dir ID (requires --token) · 1fichier: folder ID
        #[arg(short, long)]
        folder: Option<String>,
    },

    /// Show upload limits and expiry for all providers
    Providers,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Providers => print_providers(),

        Commands::Download { url, output, token } => {
            let token = resolve_download_token(&url, token);
            let path = download::download(&url, output.as_deref(), token.as_deref()).await?;
            println!("Saved to {}", path.display());
        }

        Commands::Upload {
            file,
            provider,
            token,
            note,
            folder,
        } => {
            if !file.exists() {
                anyhow::bail!("File not found: {}", file.display());
            }
            if !file.is_file() {
                anyhow::bail!("Not a regular file: {}", file.display());
            }

            let provider_name = canonical_provider(&provider);
            let token = resolve_upload_token(provider_name, token);

            let url = match provider_name {
                "fuckingfast" => {
                    FuckingFast::new(token.clone())
                        .upload(&file, note.as_deref(), folder.as_deref())
                        .await?
                }
                "gofile" => {
                    Gofile::new(token.clone())
                        .upload(&file, folder.as_deref())
                        .await?
                }
                "pixeldrain" => Pixeldrain::new(token.clone()).upload(&file).await?,
                "vikingfile" => {
                    VikingFile::new(token.clone())
                        .upload(&file, folder.as_deref())
                        .await?
                }
                "rootz" => {
                    Rootz::new(token.clone())
                        .upload(&file, folder.as_deref())
                        .await?
                }
                "1fichier" => {
                    OneFichier::new(token.clone())
                        .upload(&file, folder.as_deref())
                        .await?
                }
                _ => anyhow::bail!(
                    "Unknown provider '{}'. Run 'fsc providers' to see all options.",
                    provider
                ),
            };

            println!("{}", url);
        }
    }

    Ok(())
}

fn print_providers() {
    // columns: name · alias · size limit · anon expiry · auth expiry
    // columns: name · alias · size limit · anon expiry · auth expiry · download
    let rows: &[(&str, &str, &str, &str, &str, &str)] = &[
        (
            "gofile",
            "gf",
            "unlimited",
            "10 days (resets on download)",
            "permanent (premium)",
            "yes",
        ),
        (
            "fuckingfast",
            "ff",
            "unlimited",
            "deleted if <30 downloads in 60 days",
            "same",
            "no",
        ),
        (
            "pixeldrain",
            "pd",
            "unlimited (API key required)",
            "60 days inactivity (resets on download)",
            "same",
            "yes",
        ),
        (
            "vikingfile",
            "vf",
            "unlimited",
            "15 days after last download",
            "never (premium)",
            "no",
        ),
        (
            "rootz",
            "rz",
            "25 GB (anon) / unlimited (auth)",
            "15 days",
            "no expiry",
            "yes *",
        ),
        (
            "1fichier",
            "1f",
            "300 GB/file · 500 GB/upload",
            "no stated expiry",
            "permanent",
            "yes ** (Premium)",
        ),
    ];

    let headers = [
        "PROVIDER",
        "ALIAS",
        "SIZE LIMIT",
        "ANON EXPIRY",
        "AUTH EXPIRY",
        "DOWNLOAD",
    ];
    let mut widths = [0usize; 6];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = h.len();
    }
    for (name, alias, size, anon, auth, dl) in rows {
        let cols = [*name, *alias, *size, *anon, *auth, *dl];
        for (i, col) in cols.iter().enumerate() {
            widths[i] = widths[i].max(col.len());
        }
    }

    let [w0, w1, w2, w3, w4, _] = widths;
    println!(
        "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
        headers[0],
        headers[1],
        headers[2],
        headers[3],
        headers[4],
        headers[5],
        w0 = w0,
        w1 = w1,
        w2 = w2,
        w3 = w3,
        w4 = w4
    );
    println!("{}", "-".repeat(widths.iter().sum::<usize>() + 5 * 2));
    for (name, alias, size, anon, auth, dl) in rows {
        println!(
            "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
            name,
            alias,
            size,
            anon,
            auth,
            dl,
            w0 = w0,
            w1 = w1,
            w2 = w2,
            w3 = w3,
            w4 = w4
        );
    }
    println!("\n* rootz download uses the file UUID; may not work for all share links");
    println!("** 1fichier download requires a Premium account API key");
}

/// Resolve short aliases to canonical provider names.
fn canonical_provider(name: &str) -> &str {
    match name.trim().to_ascii_lowercase().as_str() {
        "ff" => "fuckingfast",
        "gf" => "gofile",
        "pd" => "pixeldrain",
        "vf" => "vikingfile",
        "rz" => "rootz",
        "1f" => "1fichier",
        "fuckingfast" => "fuckingfast",
        "gofile" => "gofile",
        "pixeldrain" => "pixeldrain",
        "vikingfile" => "vikingfile",
        "rootz" => "rootz",
        "1fichier" => "1fichier",
        _ => name,
    }
}

fn resolve_upload_token(provider: &str, cli_token: Option<String>) -> Option<String> {
    token::normalize(cli_token).or_else(|| {
        let keys: &[&str] = match provider {
            "gofile" => &["FSC_GOFILE_TOKEN", "FSC_TOKEN"],
            "fuckingfast" => &["FSC_FUCKINGFAST_TOKEN", "FSC_TOKEN"],
            "pixeldrain" => &["FSC_PIXELDRAIN_TOKEN", "FSC_TOKEN"],
            "vikingfile" => &["FSC_VIKINGFILE_TOKEN", "FSC_TOKEN"],
            "rootz" => &["FSC_ROOTZ_TOKEN", "FSC_TOKEN"],
            "1fichier" => &["FSC_1FICHIER_TOKEN", "FSC_TOKEN"],
            _ => &["FSC_TOKEN"],
        };
        first_env_token(keys)
    })
}

fn resolve_download_token(url: &str, cli_token: Option<String>) -> Option<String> {
    token::normalize(cli_token).or_else(|| {
        let keys: &[&str] = match infer_download_provider(url) {
            Some("gofile") => &["FSC_GOFILE_TOKEN", "FSC_TOKEN"],
            Some("pixeldrain") => &["FSC_PIXELDRAIN_TOKEN", "FSC_TOKEN"],
            Some("rootz") => &["FSC_ROOTZ_TOKEN", "FSC_TOKEN"],
            Some("1fichier") => &["FSC_1FICHIER_TOKEN", "FSC_TOKEN"],
            _ => &["FSC_TOKEN"],
        };
        first_env_token(keys)
    })
}

fn first_env_token(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key)
            .ok()
            .and_then(|value| token::normalize(Some(value)))
    })
}

fn infer_download_provider(url: &str) -> Option<&'static str> {
    let lower = url.to_ascii_lowercase();

    if lower.contains("gofile.io") {
        return Some("gofile");
    }
    if lower.contains("pixeldrain.com") {
        return Some("pixeldrain");
    }
    if lower.contains("rootz.so") {
        return Some("rootz");
    }
    if lower.contains("1fichier.com")
        || lower.contains("alterupload.com")
        || lower.contains("cjoint.net")
        || lower.contains("desfichiers.com")
        || lower.contains("dfichiers.com")
        || lower.contains("megadl.fr")
        || lower.contains("mesfichiers.org")
        || lower.contains("piecejointe.net")
        || lower.contains("pjointe.com")
        || lower.contains("tenvoi.com")
        || lower.contains("dl4free.com")
    {
        return Some("1fichier");
    }

    None
}
