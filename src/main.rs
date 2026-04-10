use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod download;
mod progress;
mod providers;

use providers::{
    fuckingfast::FuckingFast, gofile::Gofile, pixeldrain::Pixeldrain, rootz::Rootz,
    vikingfile::VikingFile,
};

#[derive(Parser)]
#[command(name = "fsc", about = "Upload files to temporary file sharing platforms", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download a file from a URL or provider share link
    Download {
        /// URL or share link to download (supports gofile, pixeldrain, vikingfile, fuckingfast, rootz)
        url: String,

        /// Output file path (default: derived from URL or Content-Disposition header)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// API token (required for private gofile folders)
        #[arg(short, long)]
        token: Option<String>,
    },

    /// Upload a file
    Upload {
        /// Path to the file to upload
        file: PathBuf,

        /// Provider: gofile (gf), fuckingfast (ff), pixeldrain (pd), vikingfile (vf), rootz (rz)
        #[arg(short, long, default_value = "gofile")]
        provider: String,

        /// API token — gofile: API token · fuckingfast: account ID ·
        /// pixeldrain: API key · vikingfile: user hash · rootz: API key
        #[arg(short, long)]
        token: Option<String>,

        /// Attach a note (fuckingfast only)
        #[arg(short, long)]
        note: Option<String>,

        /// Folder — gofile/rootz: folder ID · vikingfile: path e.g. "Videos/Clips" ·
        /// fuckingfast: parent dir ID (requires --token)
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

            let url = match canonical_provider(&provider) {
                "fuckingfast" => {
                    FuckingFast::new(token)
                        .upload(&file, note.as_deref(), folder.as_deref())
                        .await?
                }
                "gofile" => Gofile::new(token).upload(&file, folder.as_deref()).await?,
                "pixeldrain" => Pixeldrain::new(token).upload(&file).await?,
                "vikingfile" => {
                    VikingFile::new(token)
                        .upload(&file, folder.as_deref())
                        .await?
                }
                "rootz" => Rootz::new(token).upload(&file, folder.as_deref()).await?,
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
        ("gofile",      "gf", "unlimited",                      "10 days (resets on download)",          "permanent (premium)", "yes"),
        ("fuckingfast", "ff", "unlimited",                      "deleted if <30 downloads in 60 days",   "same",                "no"),
        ("pixeldrain",  "pd", "unlimited (API key required)",    "60 days inactivity (resets on download)","same",               "yes"),
        ("vikingfile",  "vf", "unlimited",                      "15 days after last download",           "never (premium)",     "no"),
        ("rootz",       "rz", "25 GB (anon) / unlimited (auth)","15 days",                               "no expiry",           "yes *"),
    ];

    let headers = ["PROVIDER", "ALIAS", "SIZE LIMIT", "ANON EXPIRY", "AUTH EXPIRY", "DOWNLOAD"];
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
    println!("{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
        headers[0], headers[1], headers[2], headers[3], headers[4], headers[5],
        w0=w0, w1=w1, w2=w2, w3=w3, w4=w4);
    println!("{}", "-".repeat(widths.iter().sum::<usize>() + 5 * 2));
    for (name, alias, size, anon, auth, dl) in rows {
        println!("{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  {}",
            name, alias, size, anon, auth, dl,
            w0=w0, w1=w1, w2=w2, w3=w3, w4=w4);
    }
    println!("\n* rootz download uses the file UUID; may not work for all share links");
}

/// Resolve short aliases to canonical provider names.
fn canonical_provider(name: &str) -> &str {
    match name {
        "ff" => "fuckingfast",
        "gf" => "gofile",
        "pd" => "pixeldrain",
        "vf" => "vikingfile",
        "rz" => "rootz",
        other => other,
    }
}
