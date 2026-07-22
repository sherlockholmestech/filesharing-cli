use anyhow::Result;
use clap::{Parser, Subcommand};
use dialoguer::FuzzySelect;
use owo_colors::OwoColorize;
use std::{env, path::PathBuf};

mod download;
mod http;
mod progress;
mod providers;
mod qr;
mod settings;
mod style;
mod token;

use providers::{
    catbox::Catbox, fichier::OneFichier, fuckingfast::FuckingFast, gofile::Gofile,
    litterbox::Litterbox, pixeldrain::Pixeldrain, rootz::Rootz, temp_sh::TempSh,
    tmpfiles::TmpFiles, uguu::Uguu, vikingfile::VikingFile,
};

const DEFAULT_PROVIDER: &str = "gofile";
const PROVIDER_NAMES: &[&str] = &[
    "catbox",
    "fuckingfast",
    "gofile",
    "litterbox",
    "pixeldrain",
    "vikingfile",
    "rootz",
    "1fichier",
    "uguu",
    "tmpfiles",
    "temp.sh",
];

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
        /// URL or share link to download (supports catbox, gofile, litterbox, pixeldrain, rootz, 1fichier; others: use direct URL)
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

        /// Provider name or alias (run `fsc providers` for the full list)
        #[arg(short, long)]
        provider: Option<String>,

        /// Skip the provider picker and use gofile when --provider is omitted
        #[arg(long)]
        no_provider_prompt: bool,

        /// API token — gofile: API token · fuckingfast: account ID ·
        /// pixeldrain: API key · vikingfile: user hash · rootz: API key ·
        /// 1fichier: API key · catbox: userhash (optional) · litterbox: not needed
        /// (prefer FSC_TOKEN or provider-specific FSC_*_TOKEN env vars)
        #[arg(short, long)]
        token: Option<String>,

        /// Attach a note (fuckingfast only)
        #[arg(short, long)]
        note: Option<String>,

        /// Folder — gofile/rootz: folder ID · vikingfile: path e.g. "Videos/Clips" ·
        /// fuckingfast: parent dir ID (requires --token) · 1fichier: folder ID
        #[arg(short, long)]
        folder: Option<String>,

        /// Print a QR code and save qr_[filename].png beside the uploaded file
        #[arg(long)]
        qr: bool,
    },

    /// Show upload limits and expiry for all providers
    Providers,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Err(err) = run(cli).await {
        eprintln!("{}", style::error(err.to_string()));
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Providers => {
            print_providers();
        }

        Commands::Download { url, output, token } => {
            let token = resolve_download_token(&url, token);
            let path = download::download(&url, output.as_deref(), token.as_deref()).await?;
            println!(
                "{} {} {}",
                style::ok_prefix(),
                "Saved".bold(),
                style::dim(format!("{}", path.display()))
            );
        }

        Commands::Upload {
            file,
            provider,
            no_provider_prompt,
            token,
            note,
            folder,
            qr,
        } => {
            if !file.exists() {
                anyhow::bail!("File not found: {}", file.display());
            }
            if !file.is_file() {
                anyhow::bail!("Not a regular file: {}", file.display());
            }

            let provider = resolve_provider(provider, no_provider_prompt)?;
            let provider_name = canonical_provider(&provider)?;
            let token = resolve_upload_token(provider_name, token);

            let url = match provider_name {
                "catbox" => Catbox::new(token.clone()).upload(&file).await?,
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
                "litterbox" => {
                    Litterbox::new(token.clone())
                        .upload(&file, folder.as_deref())
                        .await?
                }
                "pixeldrain" => Pixeldrain::new(token.clone()).upload(&file).await?,
                "tmpfiles" => TmpFiles::new().upload(&file).await?,
                "temp.sh" => TempSh::new().upload(&file).await?,
                "uguu" => Uguu::new().upload(&file).await?,
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
                _ => unreachable!("canonical_provider returned unknown name"),
            };

            println!(
                "{} {} {}",
                style::ok_prefix(),
                "Uploaded to".bold(),
                style::url(&url)
            );
            if qr {
                let qr_path = qr::generate(&url, &file)?;
                println!(
                    "{} {} {}",
                    style::ok_prefix(),
                    "QR saved to".bold(),
                    style::dim(qr_path.display().to_string())
                );
            }
        }
    }

    Ok(())
}

fn print_providers() {
    use comfy_table::{ContentArrangement, Table, modifiers::UTF8_ROUND_CORNERS};

    let mut table = Table::new();
    table
        .set_content_arrangement(ContentArrangement::Dynamic)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_header([
            "Provider",
            "Alias",
            "Description",
            "Size limit",
            "Anon expiry",
            "Auth expiry",
            "Download",
        ]);

    let rows: &[(&str, &str, &str, &str, &str, &str, &str)] = &[
        (
            "catbox",
            "cb",
            "Permanent simple hosting",
            "200 MB",
            "permanent",
            "permanent (with account)",
            "yes",
        ),
        (
            "fuckingfast",
            "ff",
            "Fast large-file sharing",
            "unlimited",
            "deleted if <30 downloads in 60 days",
            "same",
            "no",
        ),
        (
            "gofile",
            "gf",
            "Folders and account storage",
            "unlimited",
            "10 days (resets on download)",
            "permanent (premium)",
            "yes",
        ),
        (
            "litterbox",
            "lb",
            "Short-lived Catbox uploads",
            "unlimited",
            "1h / 12h / 24h / 72h",
            "same",
            "yes",
        ),
        (
            "pixeldrain",
            "pd",
            "Large files with API access",
            "unlimited (API key required)",
            "60 days inactivity (resets on download)",
            "same",
            "yes",
        ),
        (
            "vikingfile",
            "vf",
            "Large temporary hosting",
            "unlimited",
            "15 days after last download",
            "never (premium)",
            "no",
        ),
        (
            "rootz",
            "rz",
            "Folders and resumable uploads",
            "25 GB (anon) / unlimited (auth)",
            "15 days",
            "no expiry",
            "yes *",
        ),
        (
            "1fichier",
            "1f",
            "Very large long-term files",
            "300 GB/file · 500 GB/upload",
            "no stated expiry",
            "permanent",
            "yes ** (Premium)",
        ),
        (
            "uguu",
            "ug",
            "Anonymous three-hour links",
            "128 MiB",
            "3 hours",
            "n/a",
            "yes",
        ),
        (
            "tmpfiles",
            "tf",
            "Configurable short-lived links",
            "100 MiB",
            "1 hour default",
            "n/a",
            "share page",
        ),
        (
            "temp.sh",
            "ts",
            "Large three-day transfers",
            "4 GB",
            "3 days",
            "n/a",
            "share page",
        ),
    ];

    for (name, alias, description, size, anon, auth, dl) in rows {
        table.add_row([*name, *alias, *description, *size, *anon, *auth, *dl]);
    }

    println!("{table}");
    println!(
        "{}",
        style::dim("* rootz download uses the file UUID; may not work for all share links")
    );
    println!(
        "{}",
        style::dim("** 1fichier download requires a Premium account API key")
    );
}

/// Resolve short aliases to canonical provider names.
fn canonical_provider(name: &str) -> Result<&str> {
    Ok(match name.trim().to_ascii_lowercase().as_str() {
        "cb" => "catbox",
        "ff" => "fuckingfast",
        "gf" => "gofile",
        "lb" => "litterbox",
        "pd" => "pixeldrain",
        "vf" => "vikingfile",
        "rz" => "rootz",
        "1f" => "1fichier",
        "ug" => "uguu",
        "tf" => "tmpfiles",
        "ts" => "temp.sh",
        "catbox" => "catbox",
        "fuckingfast" => "fuckingfast",
        "gofile" => "gofile",
        "litterbox" => "litterbox",
        "pixeldrain" => "pixeldrain",
        "vikingfile" => "vikingfile",
        "rootz" => "rootz",
        "1fichier" => "1fichier",
        "uguu" => "uguu",
        "tmpfiles" | "tmpfiles.org" => "tmpfiles",
        "temp" | "temp.sh" => "temp.sh",
        _ => anyhow::bail!(
            "Unknown provider '{}'. Run 'fsc providers' to see all options.",
            name
        ),
    })
}

fn resolve_provider(provider: Option<String>, no_provider_prompt: bool) -> Result<String> {
    if let Some(provider) = provider {
        return Ok(provider);
    }
    if no_provider_prompt {
        return Ok(DEFAULT_PROVIDER.to_string());
    }

    let default = PROVIDER_NAMES
        .iter()
        .position(|provider| *provider == DEFAULT_PROVIDER)
        .unwrap_or(0);
    let selected = FuzzySelect::new()
        .with_prompt("Select a provider")
        .items(PROVIDER_NAMES)
        .default(default)
        .interact()?;

    Ok(PROVIDER_NAMES[selected].to_string())
}

fn resolve_upload_token(provider: &str, cli_token: Option<String>) -> Option<String> {
    token::normalize(cli_token).or_else(|| {
        let keys: &[&str] = match provider {
            "catbox" => &["FSC_CATBOX_TOKEN", "FSC_TOKEN"],
            "fuckingfast" => &["FSC_FUCKINGFAST_TOKEN", "FSC_TOKEN"],
            "gofile" => &["FSC_GOFILE_TOKEN", "FSC_TOKEN"],
            "litterbox" => &["FSC_LITTERBOX_TOKEN", "FSC_TOKEN"],
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
            Some("catbox") => &["FSC_CATBOX_TOKEN", "FSC_TOKEN"],
            Some("gofile") => &["FSC_GOFILE_TOKEN", "FSC_TOKEN"],
            Some("litterbox") => &["FSC_LITTERBOX_TOKEN", "FSC_TOKEN"],
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

    if lower.contains("catbox.moe") {
        return Some("catbox");
    }
    if lower.contains("gofile.io") {
        return Some("gofile");
    }
    if lower.contains("litterbox.catbox.moe") {
        return Some("litterbox");
    }
    if lower.contains("pixeldrain.com") {
        return Some("pixeldrain");
    }
    if lower.contains("rootz.so") {
        return Some("rootz");
    }
    if download::is_1fichier_url(&lower) {
        return Some("1fichier");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PROVIDER, canonical_provider, resolve_provider};

    #[test]
    fn explicit_provider_skips_prompt() {
        assert_eq!(
            resolve_provider(Some("ff".to_string()), false).unwrap(),
            "ff"
        );
    }

    #[test]
    fn suppressed_prompt_uses_default_provider() {
        assert_eq!(
            resolve_provider(None, true).unwrap(),
            DEFAULT_PROVIDER.to_string()
        );
    }

    #[test]
    fn resolves_new_provider_aliases() {
        assert_eq!(canonical_provider("ug").unwrap(), "uguu");
        assert_eq!(canonical_provider("tf").unwrap(), "tmpfiles");
        assert_eq!(canonical_provider("ts").unwrap(), "temp.sh");
    }
}
