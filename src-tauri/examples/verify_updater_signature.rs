use std::env;
use std::fs;
use std::path::Path;

use base64::Engine;
use minisign_verify::{PublicKey, Signature};

fn decode_tauri_value(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.starts_with("untrusted comment: ") {
        return Ok(trimmed.to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .map_err(|_| format!("{label} is not valid base64"))?;
    String::from_utf8(bytes).map_err(|_| format!("{label} is not UTF-8 minisign data"))
}

fn verify(artifact: &Path, signature_path: &Path, public_key: &str) -> Result<(), String> {
    let key_text = decode_tauri_value(public_key, "public key")?;
    let signature_text = decode_tauri_value(
        &fs::read_to_string(signature_path).map_err(|_| "unable to read signature".to_owned())?,
        "signature",
    )?;
    let key = PublicKey::decode(&key_text).map_err(|_| "public key is invalid".to_owned())?;
    let signature = Signature::decode(&signature_text)
        .map_err(|_| "updater signature is invalid".to_owned())?;
    let bytes = fs::read(artifact).map_err(|_| "unable to read artifact".to_owned())?;
    key.verify(&bytes, &signature, false)
        .map_err(|_| "updater signature does not match artifact".to_owned())
}

fn main() {
    let arguments = env::args().collect::<Vec<_>>();
    if arguments.len() != 4 {
        eprintln!("usage: verify_updater_signature <artifact> <signature> <public-key>");
        std::process::exit(2);
    }
    if let Err(error) = verify(
        Path::new(&arguments[1]),
        Path::new(&arguments[2]),
        &arguments[3],
    ) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    println!("passed");
}
