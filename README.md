# fsc

A command-line tool for uploading files to temporary file sharing platforms.

## Install

```sh
cargo install --path .
```

## Commands

### Upload

```sh
fsc upload <FILE> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--provider <NAME>` | `-p` | Provider to use (default: `gofile`) |
| `--token <TOKEN>` | `-t` | API token for authenticated uploads |
| `--folder <ID>` | `-f` | Folder/directory to upload into |
| `--note <TEXT>` | `-n` | Attach a note to the file (fuckingfast only) |

```sh
# Anonymous upload to gofile (default)
fsc upload photo.jpg

# Upload to fuckingfast
fsc upload video.mp4 --provider ff

# Authenticated upload to rootz
fsc upload archive.zip --provider rz --token YOUR_API_KEY

# Upload into a folder
fsc upload file.txt --provider gf --token YOUR_TOKEN --folder FOLDER_ID

# Upload with a note (fuckingfast)
fsc upload notes.pdf --provider ff --note "draft, expires soon"
```

### Download

```sh
fsc download <URL> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--output <PATH>` | `-o` | Save to a specific path (default: filename from URL or headers) |
| `--token <TOKEN>` | `-t` | API token (required for private gofile folders) |

```sh
# Download from a share link
fsc download https://gofile.io/d/abc123
fsc download https://pixeldrain.com/u/abc123
fsc download https://rootz.so/d/abc123

# Save to a specific path
fsc download https://gofile.io/d/abc123 --output ./myfile.zip

# Download a private gofile folder
fsc download https://gofile.io/d/abc123 --token YOUR_TOKEN

# Download any direct URL
fsc download https://example.com/file.zip
```

### Providers

```sh
fsc providers
```

Lists all supported providers with their size limits, expiry policies, and download support.

## Providers

| Provider | Alias | Size limit | Anon expiry | Auth expiry | Download |
|----------|-------|-----------|-------------|-------------|----------|
| gofile | `gf` | unlimited | 10 days (resets on download) | permanent (premium) | yes |
| fuckingfast | `ff` | unlimited | deleted if <30 downloads in 60 days | same | no |
| pixeldrain | `pd` | unlimited | 60 days inactivity (resets on download) | same | yes |
| vikingfile | `vf` | unlimited | 15 days after last download | never (premium) | no |
| rootz | `rz` | 25 GB (anon) / unlimited (auth) | 15 days | no expiry | yes * |

\* rootz download uses the file UUID; may not work for all share links.

### Token reference

| Provider | `--token` value | Where to get it |
|----------|----------------|-----------------|
| gofile | API token | Profile page on gofile.io |
| fuckingfast | Account ID | Dashboard on fuckingfast.net |
| pixeldrain | API key | [pixeldrain.com/user/api_keys](https://pixeldrain.com/user/api_keys) |
| vikingfile | User hash | Dashboard on vikingfile.com |
| rootz | API key | Dashboard settings on rootz.so |

## Upload behaviour

- **Progress bar** — shows bytes transferred, speed, and ETA for all uploads
- **gofile / vikingfile / rootz** — use multipart uploads with parallel streams for large files (up to 6 concurrent chunks, adaptive based on file size)
- **fuckingfast / pixeldrain** — single streaming PUT

## Download behaviour

| Provider | Method |
|----------|--------|
| pixeldrain | Constructs direct API download URL from share link |
| gofile | Resolves file URL via `api.gofile.io/contents/{code}` |
| rootz | Resolves signed URL via `rootz.so/api/files/download/{id}` |
| fuckingfast | No download API — not supported |
| vikingfile | No download API — not supported |
| Any URL | Direct download with redirect following |
