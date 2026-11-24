//! Log append-only de recibos, encadenado por hash.
//!
//! Cada línea es un `LogEntry` con su `seq` y el hash de la entrada anterior.
//! Alterar o quitar una línea rompe la cadena y `verify` lo detecta.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Receipt, ReceiptError};

pub const LOG_SCHEMA: &str = "navegador.receipt-log/1";
pub const GENESIS: &str = "genesis";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub schema: String,
    pub seq: u64,
    pub prev: String,
    pub receipt: Receipt,
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("esquema de log desconocido: {0}")]
    Schema(String),
    #[error("secuencia rota: se esperaba {expected}, se encontró {found}")]
    Sequence { expected: u64, found: u64 },
    #[error("cadena rota en la entrada {seq}")]
    Chain { seq: u64 },
    #[error("recibo inválido en la entrada {seq}: {source}")]
    Receipt {
        seq: u64,
        #[source]
        source: ReceiptError,
    },
    #[error("E/S del log: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON inválido en el log: {0}")]
    Json(String),
}

impl LogEntry {
    /// Hash canónico de la entrada, que la siguiente usa como `prev`.
    pub fn hash(&self) -> String {
        let value = serde_json::json!({
            "schema": self.schema,
            "seq": self.seq,
            "prev": self.prev,
            "receipt": self.receipt,
        });
        hex::encode(Sha256::digest(
            descriptor::canonical_json(&value).as_bytes(),
        ))
    }
}

/// Log en memoria respaldado por un fichero JSONL.
pub struct ReceiptLog {
    path: PathBuf,
    entries: Vec<LogEntry>,
}

impl ReceiptLog {
    /// Abre (o crea) el log y verifica lo que ya hubiera.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, LogError> {
        let path = path.into();
        match File::open(&path) {
            Ok(file) => Self::read(path, file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path,
                entries: Vec::new(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    fn read(path: PathBuf, file: File) -> Result<Self, LogError> {
        let mut entries = Vec::new();
        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry = serde_json::from_str(&line).map_err(|e| LogError::Json(e.to_string()))?;
            entries.push(entry);
        }
        let log = Self { path, entries };
        log.verify()?;
        Ok(log)
    }

    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn last_receipt(&self) -> Option<&Receipt> {
        self.entries.last().map(|entry| &entry.receipt)
    }

    /// Añade un recibo ya verificado y devuelve la entrada escrita.
    pub fn append(&mut self, receipt: Receipt) -> Result<&LogEntry, LogError> {
        crate::verify(&receipt).map_err(|source| LogError::Receipt {
            seq: self.entries.len() as u64,
            source,
        })?;

        let seq = self.entries.len() as u64;
        let prev = self
            .entries
            .last()
            .map(LogEntry::hash)
            .unwrap_or_else(|| GENESIS.to_string());
        let entry = LogEntry {
            schema: LOG_SCHEMA.to_string(),
            seq,
            prev,
            receipt,
        };

        let mut line = serde_json::to_string(&entry).map_err(|e| LogError::Json(e.to_string()))?;
        line.push('\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.flush()?;

        self.entries.push(entry);
        Ok(self.entries.last().expect("recién añadida"))
    }

    /// Verifica secuencia, cadena y cada recibo.
    pub fn verify(&self) -> Result<(), LogError> {
        let mut prev = GENESIS.to_string();
        for (index, entry) in self.entries.iter().enumerate() {
            let expected = index as u64;
            if entry.schema != LOG_SCHEMA {
                return Err(LogError::Schema(entry.schema.clone()));
            }
            if entry.seq != expected {
                return Err(LogError::Sequence {
                    expected,
                    found: entry.seq,
                });
            }
            if entry.prev != prev {
                return Err(LogError::Chain { seq: entry.seq });
            }
            crate::verify(&entry.receipt).map_err(|source| LogError::Receipt {
                seq: entry.seq,
                source,
            })?;
            prev = entry.hash();
        }
        Ok(())
    }
}

/// Verifica un fichero de log sin abrirlo para escritura.
pub fn verify_file(path: &Path) -> Result<usize, LogError> {
    let log = ReceiptLog::read(path.to_path_buf(), File::open(path)?)?;
    Ok(log.entries().len())
}
