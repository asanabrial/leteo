//! Getting the model onto a machine that arrived without it.
//!
//! One path, for every install that has no model beside it: `cargo install`, a
//! build from source, a distro package, a manager nobody has written yet.
//! Nothing here knows or asks which of them put the binary where it is. A release
//! archive carries the model and never comes this way; this is for the rest.
//!
//! Both sources end in the same place and the same way. Every file is checked
//! against the hashes compiled into the binary *before* anything is written where
//! the model is looked for, the three go into a directory of their own, and that
//! directory is renamed into place, so a download that stops half-way, a hash that
//! does not match or a source that is not the model leaves what was there
//! untouched.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::{Directory, MODEL_FILES, inspect, sha256_hex};

/// What a release calls a model file: the model's own name, prefixed, because
/// release assets share one flat namespace with the archives.
const ASSET_PREFIX: &str = "leteo-model-";

/// The environment variable that replaces [`release_base`], for a mirror or a
/// test. Documented in `cli.md`.
pub const RELEASE_URL_ENV: &str = "LETEO_MODEL_URL";

/// How long a download may take in all. The model is 13 MB.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The directory of the release that matches this binary: the model files are
/// assets of the release whose tag is the crate's version.
///
/// The hashes compiled into the binary decide whether what comes back is the
/// model, so the URL only has to be a place to ask. A release that does not carry
/// the files answers 404 and the install says so.
pub fn release_base(overridden: Option<&str>) -> String {
    overridden
        .map(str::to_owned)
        .or_else(|| {
            std::env::var(RELEASE_URL_ENV)
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| {
            format!(
                "{}/releases/download/v{}",
                env!("CARGO_PKG_REPOSITORY").trim_end_matches('/'),
                env!("CARGO_PKG_VERSION")
            )
        })
}

/// The address of one model file under a release directory.
pub fn asset_url(base: &str, name: &str) -> String {
    format!("{}/{ASSET_PREFIX}{name}", base.trim_end_matches('/'))
}

/// What an install put where.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Installed {
    pub directory: PathBuf,
    pub files: Vec<String>,
}

/// Installs from a directory that holds the three files, for a machine with no
/// network: a release archive's `model/`, or a copy made elsewhere.
pub fn from_directory(source: &Path, data_dir: &Path) -> Result<Installed> {
    let bytes = match inspect(source) {
        Directory::Verified(bytes) => bytes,
        Directory::Empty => bail!("{} holds none of the model files", source.display()),
        Directory::Wrong(problems) => bail!(
            "{} is not the model this build accepts: {}",
            source.display(),
            problems.join("; ")
        ),
    };
    place(data_dir, bytes)
}

/// Downloads the three files from `base` and installs them.
pub async fn from_release(base: &str, data_dir: &Path) -> Result<Installed> {
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(concat!("leteo/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("could not build an HTTP client")?;
    let mut fetched = Vec::new();
    for (name, expected) in MODEL_FILES {
        let url = asset_url(base, name);
        let response = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("could not reach {url}"))?;
        if !response.status().is_success() {
            bail!("{url} answered {}", response.status());
        }
        let body = response
            .bytes()
            .await
            .with_context(|| format!("the download of {url} did not finish"))?
            .to_vec();
        if sha256_hex(&body) != expected {
            bail!(
                "{url} is not the file this build accepts ({name} does not match its pinned SHA-256); nothing was installed"
            );
        }
        fetched.push(body);
    }
    place(
        data_dir,
        <[Vec<u8>; 3]>::try_from(fetched).expect("one body per pinned file"),
    )
}

/// Writes already-verified bytes into `<data dir>/model/`, all or nothing.
///
/// Written beside the destination under a name of its own and renamed over it, so
/// a reader never sees a model with some files new and some old. The previous
/// directory is moved aside first and put back if the rename fails; between the
/// two renames there is a moment with no model, and a search in it is a lexical
/// search, which is what it was before.
fn place(data_dir: &Path, bytes: [Vec<u8>; 3]) -> Result<Installed> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("could not create {}", data_dir.display()))?;
    let staging = data_dir.join(format!("model.new-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .with_context(|| format!("could not create {}", staging.display()))?;
    let outcome = (|| -> Result<()> {
        for ((name, _), body) in MODEL_FILES.iter().zip(&bytes) {
            std::fs::write(staging.join(name), body)
                .with_context(|| format!("could not write {name}"))?;
        }
        // The files on disk, not the bytes in memory: what was written is what
        // is checked, so a full disk or a bad write is caught here.
        match inspect(&staging) {
            Directory::Verified(_) => Ok(()),
            _ => bail!("the files written to {} do not verify", staging.display()),
        }
    })();
    if let Err(error) = outcome {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }

    let target = data_dir.join("model");
    let aside = data_dir.join(format!("model.old-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&aside);
    let had_one = target.exists();
    if had_one {
        std::fs::rename(&target, &aside)
            .with_context(|| format!("could not move {} aside", target.display()))?;
    }
    if let Err(error) = std::fs::rename(&staging, &target) {
        if had_one {
            let _ = std::fs::rename(&aside, &target);
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error)
            .with_context(|| format!("could not move the model into {}", target.display()));
    }
    let _ = std::fs::remove_dir_all(&aside);
    Ok(Installed {
        directory: target,
        files: MODEL_FILES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{needs_model, repository_model};
    use super::super::{Status, status};
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn copy_model(into: &Path) {
        std::fs::create_dir_all(into).unwrap();
        for (name, _) in MODEL_FILES {
            std::fs::copy(repository_model().unwrap().join(name), into.join(name)).unwrap();
        }
    }

    /// A server on a port of its own that answers `GET /<path>` from a map, for
    /// as many requests as the test makes. Never the network.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let head = String::from_utf8_lossy(&request[..read]).into_owned();
                let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                let reply = match files.iter().find(|(name, _)| *name == path) {
                    Some((_, body)) => {
                        let mut out = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        out.extend_from_slice(body);
                        out
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = stream.write_all(&reply);
            }
        });
        format!("http://{address}/v")
    }

    fn assets(corrupt: Option<&str>) -> Vec<(String, Vec<u8>)> {
        MODEL_FILES
            .iter()
            .map(|(name, _)| {
                let mut body = std::fs::read(repository_model().unwrap().join(name)).unwrap();
                if corrupt == Some(*name) {
                    body[10] ^= 0xff;
                }
                (format!("/v/{ASSET_PREFIX}{name}"), body)
            })
            .collect()
    }

    #[test]
    fn the_release_directory_is_the_one_of_this_version_and_asset_names_are_prefixed() {
        let base = release_base(None);
        assert!(
            base.ends_with(&format!(
                "/releases/download/v{}",
                env!("CARGO_PKG_VERSION")
            )),
            "{base}"
        );
        assert_eq!(
            asset_url("http://h/v/", "config.json"),
            "http://h/v/leteo-model-config.json"
        );
        assert_eq!(release_base(Some("http://mirror/x")), "http://mirror/x");
    }

    #[test]
    fn a_local_copy_installs_and_verifies() {
        let Some(_) = needs_model!() else { return };
        let scratch = tempfile::TempDir::new().unwrap();
        let (copy, data) = (scratch.path().join("copy"), scratch.path().join("data"));
        copy_model(&copy);
        let done = from_directory(&copy, &data).unwrap();
        assert_eq!(done.directory, data.join("model"));
        assert_eq!(
            status(&data, Some(Path::new("/nonexistent"))),
            Status::Verified(data.join("model"))
        );
        assert_eq!(
            std::fs::read_dir(&data).unwrap().count(),
            1,
            "no staging directory is left behind"
        );
    }

    /// The refusals, each leaving what was already installed exactly as it was.
    #[test]
    fn a_missing_file_or_a_flipped_byte_installs_nothing_and_changes_nothing() {
        let Some(_) = needs_model!() else { return };
        let scratch = tempfile::TempDir::new().unwrap();
        let (copy, data) = (scratch.path().join("copy"), scratch.path().join("data"));
        copy_model(&copy);
        from_directory(&copy, &data).unwrap();
        let before = std::fs::read(data.join("model/model.safetensors")).unwrap();

        std::fs::remove_file(copy.join("config.json")).unwrap();
        let error = from_directory(&copy, &data).unwrap_err().to_string();
        assert!(error.contains("config.json is missing"), "{error}");

        copy_model(&copy);
        let mut bytes = std::fs::read(copy.join("model.safetensors")).unwrap();
        bytes[100] ^= 0x01;
        std::fs::write(copy.join("model.safetensors"), bytes).unwrap();
        let error = from_directory(&copy, &data).unwrap_err().to_string();
        assert!(
            error.contains("model.safetensors does not match"),
            "{error}"
        );

        assert_eq!(
            std::fs::read(data.join("model/model.safetensors")).unwrap(),
            before
        );
        assert!(matches!(
            status(&data, Some(Path::new("/nonexistent"))),
            Status::Verified(_)
        ));
        assert!(from_directory(&scratch.path().join("empty"), &data).is_err());
    }

    #[tokio::test]
    async fn a_release_is_downloaded_verified_and_placed() {
        let Some(_) = needs_model!() else { return };
        let data = tempfile::TempDir::new().unwrap();
        let base = serve(assets(None));
        let done = from_release(&base, data.path()).await.unwrap();
        assert_eq!(done.files.len(), 3);
        assert!(matches!(
            status(data.path(), Some(Path::new("/nonexistent"))),
            Status::Verified(_)
        ));
    }

    #[tokio::test]
    async fn a_download_that_is_not_the_model_or_not_there_installs_nothing() {
        let Some(_) = needs_model!() else { return };
        let data = tempfile::TempDir::new().unwrap();
        let base = serve(assets(Some("model.safetensors")));
        let error = from_release(&base, data.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("does not match its pinned SHA-256"),
            "{error}"
        );
        assert!(!data.path().join("model").exists());
        assert_eq!(
            std::fs::read_dir(data.path()).unwrap().count(),
            0,
            "nothing was left behind"
        );

        let base = serve(Vec::new());
        let error = from_release(&base, data.path())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("404"), "{error}");
        assert!(!data.path().join("model").exists());
    }
}
