use std::{error::Error, fmt};

use sha2::{Digest, Sha256};

pub const APPLICATION_ID: i64 = 0x4750_5445;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
    pub checksum: &'static str,
    pub schema_fingerprint: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "0001_initial",
    sql: include_str!("0001_initial.sql"),
    checksum: "49d00e581771551697152a7a7193419d718917e74b4d32245762df6d3287e2f3",
    schema_fingerprint: "1c2e0345a5e1f7d67a4ed8388cded53bf9f343f5bdc7e4883f242720433cf7e7",
}];

pub const CURRENT_SCHEMA_VERSION: u32 = MIGRATIONS[MIGRATIONS.len() - 1].version;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryError(&'static str);

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for RegistryError {}

pub fn validate_registry() -> Result<(), RegistryError> {
    if MIGRATIONS.is_empty() {
        return Err(RegistryError("migration registry must not be empty"));
    }

    for (index, migration) in MIGRATIONS.iter().enumerate() {
        let expected_version = u32::try_from(index + 1)
            .map_err(|_| RegistryError("migration registry is too large"))?;
        if migration.version != expected_version {
            return Err(RegistryError(
                "migration versions must be continuous from one",
            ));
        }
        if migration.name.trim().is_empty()
            || MIGRATIONS[..index]
                .iter()
                .any(|previous| previous.name == migration.name)
        {
            return Err(RegistryError(
                "migration names must be non-empty and unique",
            ));
        }
        if migration.checksum != migration_checksum(migration.sql) {
            return Err(RegistryError(
                "migration checksum does not match embedded SQL",
            ));
        }
        if migration.schema_fingerprint != schema_fingerprint(migration.version, migration.checksum)
        {
            return Err(RegistryError(
                "migration schema fingerprint does not match its identity",
            ));
        }
    }

    Ok(())
}

pub fn migration_checksum(sql: &str) -> String {
    lowercase_hex(&Sha256::digest(sql.as_bytes()))
}

pub fn schema_fingerprint(version: u32, checksum: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"gpteasy-schema-fingerprint-v1\0");
    hasher.update(APPLICATION_ID.to_be_bytes());
    hasher.update(version.to_be_bytes());
    hasher.update(checksum.as_bytes());
    lowercase_hex(&hasher.finalize())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}
