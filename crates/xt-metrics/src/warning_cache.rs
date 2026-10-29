//! Saved default-threshold M-21 decisions. The compute APIs remain the oracle.
use crate::session_warnings::{ACTIVE_QUERY, NeutralWholeSessionWarnings};
use crate::{MetricsDb, Result, WholeSessionWarnings, Window, WindowSessionWarnings};
use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use xt_store::saved_session_warnings::SavedSessionWarning;

/// Bump for changes to payload, rules, default thresholds or stored-fact meaning.
/// 4: the repeated-calls rule was removed. Every answer saved at 3 or earlier
/// is out of date, so none that still holds a repeated-calls warning is shown;
/// each is recounted under the four remaining rules.
/// 5: the admitted-mistake rule was added, with its own check. Every answer
/// saved at 4 or earlier lacks it, and is recounted.
pub const SESSION_WARNINGS_CACHE_VERSION: i64 = 5;

pub(crate) const SAVED_QUERY: &str = "SELECT session_id,i.revision,c.revision,c.rule_version,c.payload_json FROM sessions s LEFT JOIN session_warning_inputs i USING(session_id) LEFT JOIN saved_session_warnings c USING(session_id) WHERE s.kind='user'";
pub(crate) const DIRTY_QUERY: &str = "SELECT session_id,i.revision FROM session_warning_inputs i JOIN sessions s USING(session_id) LEFT JOIN saved_session_warnings c USING(session_id) WHERE s.kind='user' AND (c.revision IS NOT i.revision OR c.rule_version IS NOT ?1)";

pub struct SessionWarningRefresh {
    pub rows: Vec<SavedSessionWarning>,
}

#[derive(Serialize, Deserialize)]
struct Payload {
    version: i64,
    revision: i64,
    last_ms: Option<i64>,
    answer: NeutralWholeSessionWarnings,
}

impl MetricsDb {
    /// A metadata-only check, composed in one snapshot. No history is loaded.
    pub fn session_warning_cache_is_current(&self) -> Result<bool> {
        self.read_snapshot(|db| {
            Ok(!db
                .connection
                .prepare(DIRTY_QUERY)?
                .exists([SESSION_WARNINGS_CACHE_VERSION])?)
        })
    }

    pub fn session_warning_refresh_needed(&self) -> bool {
        self.warning_refresh_needed.get()
    }
    pub fn session_warning_repairs(&self) -> Vec<String> {
        self.warning_repairs.borrow().iter().cloned().collect()
    }

    /// Prepare only stale/missing rows and explicitly unavailable payloads.
    pub fn prepare_session_warning_refresh(
        &self,
        repairs: &[String],
    ) -> Result<Option<SessionWarningRefresh>> {
        self.prepare_warning_refresh(repairs, &|| false)
    }

    pub fn prepare_session_warning_refresh_cancellable(
        &self,
        repairs: &[String],
        cancel: Arc<AtomicBool>,
    ) -> Result<Option<SessionWarningRefresh>> {
        let progress = Arc::clone(&cancel);
        self.connection
            .progress_handler(10_000, Some(move || progress.load(Ordering::Relaxed)))?;
        let result = self.prepare_warning_refresh(repairs, &|| cancel.load(Ordering::Relaxed));
        self.connection.progress_handler(0, None::<fn() -> bool>)?;
        result
    }

    fn prepare_warning_refresh(
        &self,
        repairs: &[String],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<SessionWarningRefresh>> {
        self.read_snapshot(|db| {
            if cancelled() { return Err(crate::Error::Cancelled); }
            let mut revisions=db.connection.prepare(DIRTY_QUERY)?.query_map([SESSION_WARNINGS_CACHE_VERSION],|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<BTreeMap<_,_>>>()?;
            for chunk in repairs.chunks(crate::MAX_SESSIONS) {
                let sql=crate::session::by_session_from("SELECT session_id,revision FROM session_warning_inputs JOIN sessions USING(session_id) WHERE kind='user'",chunk.len(),1);
                revisions.extend(db.connection.prepare(&sql)?.query_map(rusqlite::params_from_iter(chunk),|row|Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?)))?.collect::<rusqlite::Result<BTreeMap<_,_>>>()?);
            }
            let cleanup:bool=db.connection.query_row("SELECT EXISTS(SELECT 1 FROM saved_session_warnings WHERE session_id NOT IN (SELECT session_id FROM sessions WHERE kind='user'))",[],|r|r.get(0))?;
            if revisions.is_empty()&&!cleanup { return Ok(None); }
            let ids:Vec<_>=revisions.keys().map(String::as_str).collect();
            let (answers,spans)=db.whole_neutral_in_snapshot(&ids,cancelled)?;
            let mut rows=Vec::with_capacity(answers.len());
            for (id,answer) in answers {
                if cancelled() { return Err(crate::Error::Cancelled); }
                let revision=revisions[&id];
                let payload=Payload{version:SESSION_WARNINGS_CACHE_VERSION,revision,last_ms:spans.get(&id).map(|(_,last)|*last),answer};
                rows.push(SavedSessionWarning{session_id:id,revision,rule_version:SESSION_WARNINGS_CACHE_VERSION,payload_json:serde_json::to_string(&payload).map_err(xt_store::Error::from)?});
            }
            Ok(Some(SessionWarningRefresh{rows}))
        })
    }

    pub fn saved_whole_session_warnings(
        &self,
        sessions: &[&str],
        zone: &TimeZone,
    ) -> Result<BTreeMap<String, WholeSessionWarnings>> {
        let mut ids = sessions.to_vec();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > crate::MAX_SESSIONS {
            return Err(crate::Error::TooManySessions);
        }
        self.read_snapshot(|db| Ok(db.saved_warning_answers(&ids, zone, true)?.0))
    }

    /// [`MetricsDb::saved_whole_session_warnings`] from saved answers only,
    /// never recounting: a session whose saved answer is out of date or
    /// unusable is [`WholeSessionWarnings::Missing`] here (and marked for the
    /// background refresh as usual). For a detail that may wait for the next
    /// refresh rather than hold its caller for a whole-session recount.
    pub fn current_saved_whole_session_warnings(
        &self,
        sessions: &[&str],
        zone: &TimeZone,
    ) -> Result<BTreeMap<String, WholeSessionWarnings>> {
        let mut ids = sessions.to_vec();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() > crate::MAX_SESSIONS {
            return Err(crate::Error::TooManySessions);
        }
        self.read_snapshot(|db| Ok(db.saved_warning_answers(&ids, zone, false)?.0))
    }

    /// The saved answers for `ids`; out-of-date or unusable ones are
    /// recounted when `recount`, and left [`WholeSessionWarnings::Missing`]
    /// otherwise.
    fn saved_warning_answers(
        &self,
        ids: &[&str],
        zone: &TimeZone,
        recount: bool,
    ) -> Result<(
        BTreeMap<String, WholeSessionWarnings>,
        BTreeMap<String, i64>,
    )> {
        let mut answers: BTreeMap<_, _> = ids
            .iter()
            .map(|id| ((*id).to_owned(), WholeSessionWarnings::Missing))
            .collect();
        let mut spans = BTreeMap::new();
        let mut stale = BTreeSet::new();
        for chunk in ids.chunks(crate::MAX_SESSIONS) {
            let sql = crate::session::by_session_from(SAVED_QUERY, chunk.len(), 1);
            let mut statement = self.connection.prepare(&sql)?;
            let mut rows = statement.query(rusqlite::params_from_iter(chunk))?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let input: Option<i64> = row.get(1)?;
                let saved: Option<i64> = row.get(2)?;
                let version: Option<i64> = row.get(3)?;
                if input.is_none() {
                    return Err(
                        xt_store::Error::InvalidInput("warning source revision missing").into(),
                    );
                }
                if input != saved || version != Some(SESSION_WARNINGS_CACHE_VERSION) {
                    stale.insert(id);
                    continue;
                }
                let payload = row
                    .get::<_, Option<String>>(4)?
                    .and_then(|json| serde_json::from_str::<Payload>(&json).ok());
                match payload {
                    Some(payload)
                        if payload.version == SESSION_WARNINGS_CACHE_VERSION
                            && Some(payload.revision) == input
                            && match &payload.answer {
                                NeutralWholeSessionWarnings::Missing => false,
                                NeutralWholeSessionWarnings::Untimed { .. } => {
                                    payload.last_ms.is_none()
                                }
                                NeutralWholeSessionWarnings::Read { warnings, .. } => {
                                    warnings.is_valid_for(&id) && payload.last_ms.is_some()
                                }
                            } =>
                    {
                        if let Some(last) = payload.last_ms {
                            spans.insert(id.clone(), last);
                        }
                        answers.insert(id, payload.answer.render(zone));
                    }
                    _ => {
                        self.warning_repairs.borrow_mut().insert(id.clone());
                        stale.insert(id);
                    }
                }
            }
        }
        if !stale.is_empty() {
            self.warning_refresh_needed.set(true);
        }
        if recount && !stale.is_empty() {
            let named: Vec<_> = stale.iter().map(String::as_str).collect();
            let (fresh, fresh_spans) = self.whole_neutral_in_snapshot(&named, &|| false)?;
            answers.extend(
                fresh
                    .into_iter()
                    .map(|(id, answer)| (id, answer.render(zone))),
            );
            spans.extend(fresh_spans.into_iter().map(|(id, (_, last))| (id, last)));
        }
        Ok((answers, spans))
    }

    pub fn saved_warnings_in_window(
        &self,
        window: Window,
        zone: &TimeZone,
    ) -> Result<Vec<WindowSessionWarnings>> {
        self.read_snapshot(|db| {
            let ids = db
                .connection
                .prepare(ACTIVE_QUERY)?
                .query_map([window.start_ms(), window.candidate_end_ms()?], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let named: Vec<_> = ids.iter().map(String::as_str).collect();
            let (answers, spans) = db.saved_warning_answers(&named, zone, true)?;
            Ok(answers
                .into_iter()
                .filter_map(|(id, answer)| {
                    let WholeSessionWarnings::Read { warnings, .. } = answer else {
                        return None;
                    };
                    let warnings: Vec<_> = warnings
                        .warnings
                        .into_iter()
                        .filter(|w| w.at_ms >= window.start_ms() && w.at_ms < window.end_ms())
                        .collect();
                    (!warnings.is_empty()).then(|| WindowSessionWarnings {
                        last_ms: spans.get(&id).copied().unwrap_or(0),
                        session_id: id,
                        warnings,
                    })
                })
                .collect())
        })
    }
}

#[cfg(test)]
#[path = "warning_cache_tests.rs"]
mod tests;
