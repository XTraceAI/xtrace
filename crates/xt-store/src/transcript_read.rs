//! What the Claude `Bash` launch readers last found in a Claude transcript
//! file that held no launch (`claude_transcript_reads`, migration 33), so a
//! restart need not read an unchanged file again.
//!
//! A row is a shortcut only: the reader that saved it decides whether it
//! still describes the file (same reader version, line bound, installed
//! launchers, owner, generation and fingerprint) and reads the file as if no
//! row existed otherwise. Identifiers, a path the source cursors already
//! hold, file generations, digests and times only; never conversation
//! content, so the content purge leaves them.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

/// Which reader saved a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TranscriptReader {
    /// The literal-prompt launch match.
    Prompt,
    /// The launch reader that compares printed answers.
    Bash,
}

impl TranscriptReader {
    fn label(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Bash => "bash",
        }
    }
}

/// One saved read of a file that held no launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedTranscriptRead {
    pub reader: TranscriptReader,
    pub path: String,
    pub owner_native_session_id: String,
    pub reader_version: u32,
    pub max_line: u64,
    pub rules_digest: [u8; 32],
    pub device: i64,
    pub inode: i64,
    /// Just after the last complete line read.
    pub read_to: u64,
    /// The exact modification and change times when the read began.
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub fingerprint: [u8; 32],
    /// [`TranscriptReader::Prompt`] only: a record or line naming `claude`
    /// had no time.
    pub untimed: bool,
    /// [`TranscriptReader::Prompt`] only: when each `Bash` command naming
    /// `claude` that could not be read was made.
    pub refused_ms: Vec<i64>,
}

fn encode_times(times: &[i64]) -> Vec<u8> {
    times.iter().flat_map(|time| time.to_le_bytes()).collect()
}

fn decode_times(bytes: &[u8]) -> Option<Vec<i64>> {
    if !bytes.len().is_multiple_of(8) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(8)
            .map(|chunk| i64::from_le_bytes(chunk.try_into().expect("eight bytes")))
            .collect(),
    )
}

impl Store {
    /// The read `reader` saved for the file at `path`, if any.
    pub fn saved_transcript_read(
        &self,
        reader: TranscriptReader,
        path: &str,
    ) -> Result<Option<SavedTranscriptRead>> {
        let row = self
            .connection
            .query_row(
                "SELECT owner_native_session_id,reader_version,max_line,rules_digest,
                        device,inode,read_to,mtime_ns,ctime_ns,fingerprint,untimed,refused_ms
                   FROM claude_transcript_reads WHERE reader=?1 AND path=?2",
                params![reader.label(), path],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Vec<u8>>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, Vec<u8>>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            owner,
            version,
            max_line,
            programs,
            device,
            inode,
            read_to,
            mtime_ns,
            ctime_ns,
            fingerprint,
            untimed,
            refused,
        )) = row
        else {
            return Ok(None);
        };
        // The table's checks make these hold; a row that does not is no
        // shortcut.
        let (Ok(reader_version), Ok(max_line), Ok(read_to), Ok(rules_digest), Ok(fingerprint)) = (
            u32::try_from(version),
            u64::try_from(max_line),
            u64::try_from(read_to),
            <[u8; 32]>::try_from(programs.as_slice()),
            <[u8; 32]>::try_from(fingerprint.as_slice()),
        ) else {
            return Ok(None);
        };
        let Some(refused_ms) = decode_times(&refused) else {
            return Ok(None);
        };
        Ok(Some(SavedTranscriptRead {
            reader,
            path: path.to_owned(),
            owner_native_session_id: owner,
            reader_version,
            max_line,
            rules_digest,
            device,
            inode,
            read_to,
            mtime_ns,
            ctime_ns,
            fingerprint,
            untimed: untimed == 1,
            refused_ms,
        }))
    }

    /// Save `reads` and forget the reads of `forget`, in one transaction:
    /// each saved read replaces the one its reader saved for that file.
    pub fn save_transcript_reads(
        &mut self,
        reads: &[SavedTranscriptRead],
        forget: &[(TranscriptReader, String)],
        saved_at_ms: i64,
    ) -> Result<()> {
        if reads.is_empty() && forget.is_empty() {
            return Ok(());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO claude_transcript_reads(reader,path,owner_native_session_id,
                     reader_version,max_line,rules_digest,device,inode,read_to,mtime_ns,
                     ctime_ns,fingerprint,untimed,refused_ms,saved_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
                 ON CONFLICT(reader,path) DO UPDATE SET
                     owner_native_session_id=excluded.owner_native_session_id,
                     reader_version=excluded.reader_version,max_line=excluded.max_line,
                     rules_digest=excluded.rules_digest,device=excluded.device,
                     inode=excluded.inode,read_to=excluded.read_to,mtime_ns=excluded.mtime_ns,
                     ctime_ns=excluded.ctime_ns,fingerprint=excluded.fingerprint,
                     untimed=excluded.untimed,refused_ms=excluded.refused_ms,
                     saved_at=excluded.saved_at",
            )?;
            for read in reads {
                let max_line = i64::try_from(read.max_line)
                    .map_err(|_| Error::InvalidInput("transcript read line bound out of range"))?;
                let read_to = i64::try_from(read.read_to)
                    .map_err(|_| Error::InvalidInput("transcript read length out of range"))?;
                insert.execute(params![
                    read.reader.label(),
                    read.path,
                    read.owner_native_session_id,
                    read.reader_version,
                    max_line,
                    read.rules_digest.as_slice(),
                    read.device,
                    read.inode,
                    read_to,
                    read.mtime_ns,
                    read.ctime_ns,
                    read.fingerprint.as_slice(),
                    i64::from(read.untimed),
                    encode_times(&read.refused_ms),
                    saved_at_ms,
                ])?;
            }
            let mut delete = transaction
                .prepare("DELETE FROM claude_transcript_reads WHERE reader=?1 AND path=?2")?;
            for (reader, path) in forget {
                delete.execute(params![reader.label(), path])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}
