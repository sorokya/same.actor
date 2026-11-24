//! Persistent storage: one SQLite file, one writer thread, a small read pool.
//!
//! SQLite permits exactly one writer. Rather than configure a connection pool and hope
//! `busy_timeout` papers over the contention, a single thread owns the write connection and
//! applies batched transactions. Readers take separate connections and run concurrently with
//! the writer, which is what WAL is for.
//!
//! Writes are *offered*, not performed: [`Store::record`] drops a record when the queue is
//! full and counts the loss. Analytics must never be load-bearing, so a slow disk must not be
//! able to slow a page — the alternative is a feature that takes down the thing it measures.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tokio::sync::Semaphore;

use crate::error::AppError;
use crate::geo::Geo;

pub mod bans;
pub mod comparisons;
pub mod lists;
pub mod maintenance;
pub mod migrations;
pub mod report;
pub mod request;
pub mod searches;

pub use request::{Kind, RequestLog, RequestRow};

/// File name used when `DB_PATH` is not set. Relative on purpose: the container sets
/// `DB_PATH=/data/same-actor.db`, where `/data` is the volume.
pub const DEFAULT_PATH: &str = "data/same-actor.db";

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub path: PathBuf,
    /// How many pending records the queue holds before records start being dropped.
    pub queue_capacity: usize,
    /// Maximum rows per transaction.
    pub batch_max: usize,
    /// How long the writer waits for more rows before committing what it has.
    pub batch_wait: Duration,
    /// Read connections kept warm. WAL allows them to read during a write.
    pub read_pool: usize,
    /// Country lookup, resolved in the writer thread so the request path never pays for it.
    pub geo: Option<Arc<Geo>>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from(DEFAULT_PATH),
            queue_capacity: 8_192,
            batch_max: 256,
            batch_wait: Duration::from_millis(250),
            read_pool: 4,
            geo: None,
        }
    }
}

/// Everything the writer thread accepts. Control messages travel on the same channel so their
/// ordering relative to records is guaranteed by construction.
pub(crate) enum WriteOp {
    Request(RequestLog),
    Search(searches::SearchEvent),
    View(comparisons::NewView),
    Ban(BanWrite),
    /// `(retention_days, now, ack)`.
    Maintenance(
        u32,
        i64,
        tokio::sync::oneshot::Sender<Result<maintenance::Report, String>>,
    ),
    /// Snapshot ids to keep; everything else is dropped.
    PruneLists(
        Vec<String>,
        tokio::sync::oneshot::Sender<Result<usize, String>>,
    ),
    /// A whole discovery snapshot: `(id, payload, synergy, now, ack)`.
    PutList(
        String,
        String,
        String,
        i64,
        tokio::sync::oneshot::Sender<Result<(), String>>,
    ),
    Flush(tokio::sync::oneshot::Sender<()>),
    Shutdown(tokio::sync::oneshot::Sender<()>),
}

/// Durable ban mutations. They carry their own acknowledgement because the admin API has to
/// know the row exists before it reports success, and because the single-writer rule means the
/// handler cannot just open a connection and do it.
pub(crate) enum BanWrite {
    Insert(
        bans::NewBan,
        tokio::sync::oneshot::Sender<Result<i64, String>>,
    ),
    Delete(i64, tokio::sync::oneshot::Sender<Result<bool, String>>),
    /// Fire and forget: a refused request should not wait on a counter.
    Hit(i64),
    Purge(tokio::sync::oneshot::Sender<Result<usize, String>>),
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct StoreStats {
    pub rows: u64,
    pub commits: u64,
    /// Records discarded because the queue was full. Surfaced in the admin health panel:
    /// a non-zero value means the writer is not keeping up, and it is better to see that
    /// number than to silently lose history.
    pub dropped: u64,
}

#[derive(Default)]
struct Counters {
    rows: AtomicU64,
    commits: AtomicU64,
    dropped: AtomicU64,
}

pub struct Store {
    tx: SyncSender<WriteOp>,
    counters: Arc<Counters>,
    pool: ReadPool,
    writer: Mutex<Option<JoinHandle<()>>>,
    path: PathBuf,
}

impl Store {
    /// Open the database, apply pragmas, migrate, and start the writer thread.
    ///
    /// Synchronous on purpose: it runs once at startup, and keeping it out of the runtime means
    /// tests that only need an `AppState` do not have to be async.
    pub fn open(config: StoreConfig) -> Result<Self, AppError> {
        if let Some(parent) = config.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    AppError::Store(format!(
                        "could not create the directory for {}: {error}",
                        config.path.display()
                    ))
                })?;

                // A bind-mounted host directory is usually owned by a uid this process is not,
                // and SQLite reports that as "unable to open database file", which reads like a
                // missing file rather than a permissions problem. Say which it is.
                let probe = parent.join(".same-actor-write-probe");
                if let Err(error) = std::fs::write(&probe, b"") {
                    if error.kind() == std::io::ErrorKind::PermissionDenied {
                        return Err(AppError::Store(format!(
                            "{} exists but this process cannot write to it ({error}). If it is a \
                             bind-mounted host directory, give it to the container with \
                             `-v <dir>:/data:U`, or use a named volume instead.",
                            parent.display()
                        )));
                    }
                } else {
                    let _ = std::fs::remove_file(&probe);
                }
            }
        }

        let mut writer = open_connection(&config.path)?;
        migrations::migrate(&mut writer)?;

        let (tx, rx) = sync_channel(config.queue_capacity.max(1));
        let counters = Arc::new(Counters::default());

        let thread_counters = Arc::clone(&counters);
        let batch_max = config.batch_max.max(1);
        let batch_wait = config.batch_wait;
        let geo = config.geo.clone();
        let handle = std::thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                writer_loop(
                    &mut writer,
                    &rx,
                    &thread_counters,
                    batch_max,
                    batch_wait,
                    geo.as_deref(),
                );
            })
            .map_err(|error| AppError::Store(format!("could not start the writer: {error}")))?;

        Ok(Self {
            tx,
            counters,
            pool: ReadPool::new(config.path.clone(), config.read_pool.max(1)),
            writer: Mutex::new(Some(handle)),
            path: config.path,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn stats(&self) -> StoreStats {
        StoreStats {
            rows: self.counters.rows.load(Ordering::Relaxed),
            commits: self.counters.commits.load(Ordering::Relaxed),
            dropped: self.counters.dropped.load(Ordering::Relaxed),
        }
    }

    /// Offer a request record to the writer. Never blocks, never fails loudly: `false` means
    /// the record was dropped and counted.
    pub fn record(&self, log: RequestLog) -> bool {
        self.offer(WriteOp::Request(log))
    }

    fn offer(&self, op: WriteOp) -> bool {
        match self.tx.try_send(op) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    /// Send a durable mutation to the writer and wait for its result.
    pub(crate) async fn submit<T, F>(&self, make: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(tokio::sync::oneshot::Sender<Result<T, String>>) -> WriteOp + Send + 'static,
    {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let op = make(ack_tx);
        let tx = self.tx.clone();

        let sent = tokio::task::spawn_blocking(move || tx.send(op).is_ok())
            .await
            .map_err(|error| AppError::Store(format!("write task failed: {error}")))?;

        if !sent {
            return Err(AppError::Store("the writer has stopped".into()));
        }

        match ack_rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(AppError::Store(error)),
            Err(_) => Err(AppError::Store(
                "the writer did not acknowledge a write".into(),
            )),
        }
    }

    /// Offer a search event. Fire and forget, like request records.
    pub(crate) fn offer_search(&self, event: searches::SearchEvent) -> bool {
        self.offer(WriteOp::Search(event))
    }

    /// Offer a view of a comparison. Fire and forget.
    pub(crate) fn offer_view(&self, view: comparisons::NewView) -> bool {
        self.offer(WriteOp::View(view))
    }

    /// Count a hit against a ban without waiting for it to be written.
    pub(crate) fn offer_ban_hit(&self, id: i64) {
        self.offer(WriteOp::Ban(BanWrite::Hit(id)));
    }

    /// Wait until every record offered so far is committed. Used by tests, by the shutdown
    /// path, and by anything that needs read-after-write.
    pub async fn flush(&self) -> Result<(), AppError> {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let tx = self.tx.clone();

        // The channel may be momentarily full; the send would block, and blocking a reactor
        // thread on a log flush is exactly what this design exists to avoid.
        //
        // `is_ok()` inside the closure rather than moving the `SendError` out: that error owns
        // the whole `WriteOp`, so returning it makes the closure's `Err` type 200 bytes wide.
        let sent = tokio::task::spawn_blocking(move || tx.send(WriteOp::Flush(ack_tx)).is_ok())
            .await
            .map_err(|error| AppError::Store(format!("flush task failed: {error}")))?;

        if !sent {
            return Err(AppError::Store("the writer has stopped".into()));
        }

        ack_rx
            .await
            .map_err(|_| AppError::Store("the writer did not acknowledge a flush".into()))
    }

    /// Commit everything outstanding, checkpoint the WAL, and stop the writer thread.
    pub async fn close(&self) -> Result<(), AppError> {
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
        let tx = self.tx.clone();
        // Nothing to report if this fails: a closed channel means the writer has already
        // stopped, which is the outcome we were asking for.
        let _ =
            tokio::task::spawn_blocking(move || tx.send(WriteOp::Shutdown(ack_tx)).is_ok()).await;
        let _ = ack_rx.await;

        let handle = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();

        if let Some(handle) = handle {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }

        Ok(())
    }

    /// Run a closure against a pooled read connection, off the reactor.
    pub(crate) async fn read<T, F>(&self, f: F) -> Result<T, AppError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        self.pool.run(f).await
    }
}

fn open_connection(path: &Path) -> Result<Connection, AppError> {
    let conn = Connection::open(path)
        .map_err(|error| AppError::Store(format!("could not open {}: {error}", path.display())))?;

    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|error| AppError::Store(format!("could not set busy_timeout: {error}")))?;

    // `journal_mode` returns a row, so it cannot go through `execute_batch`.
    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(|error| AppError::Store(format!("could not enable WAL: {error}")))?;

    if !mode.eq_ignore_ascii_case("wal") {
        return Err(AppError::Store(format!(
            "the database refused WAL and stayed in {mode:?}; this design assumes one writer \
             plus concurrent readers"
        )));
    }

    conn.execute_batch(
        "PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = 268435456;
         PRAGMA wal_autocheckpoint = 1000;",
    )
    .map_err(|error| AppError::Store(format!("could not apply pragmas: {error}")))?;

    Ok(conn)
}

/// Rows waiting to be written. Two kinds, committed together: a page view and the search that
/// led to it belong in the same transaction.
#[derive(Default)]
struct Pending {
    requests: Vec<RequestLog>,
    searches: Vec<searches::SearchEvent>,
    views: Vec<comparisons::NewView>,
}

impl Pending {
    fn len(&self) -> usize {
        self.requests.len() + self.searches.len() + self.views.len()
    }

    fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.searches.is_empty() && self.views.is_empty()
    }

    fn clear(&mut self) {
        self.requests.clear();
        self.searches.clear();
        self.views.clear();
    }
}

fn writer_loop(
    conn: &mut Connection,
    rx: &Receiver<WriteOp>,
    counters: &Counters,
    batch_max: usize,
    batch_wait: Duration,
    geo: Option<&Geo>,
) {
    let mut pending = Pending {
        requests: Vec::with_capacity(batch_max),
        searches: Vec::new(),
        views: Vec::new(),
    };

    loop {
        // Block for the first item; this thread has nothing else to do.
        match rx.recv() {
            Ok(op) => {
                if !accept(op, conn, counters, &mut pending, geo) {
                    return;
                }
            }
            // Every sender is gone: commit what we have and stop.
            Err(_) => {
                commit(conn, counters, &mut pending, geo);
                return;
            }
        }

        // Drain whatever queued while we were writing, so a burst becomes one transaction.
        while pending.len() < batch_max {
            match rx.recv_timeout(batch_wait) {
                Ok(op) => {
                    if !accept(op, conn, counters, &mut pending, geo) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    commit(conn, counters, &mut pending, geo);
                    return;
                }
            }
        }

        commit(conn, counters, &mut pending, geo);
    }
}

/// Take one operation. Returns `false` when the writer has been told to stop.
fn accept(
    op: WriteOp,
    conn: &mut Connection,
    counters: &Counters,
    pending: &mut Pending,
    geo: Option<&Geo>,
) -> bool {
    match op {
        WriteOp::Request(log) => pending.requests.push(log),
        WriteOp::Search(event) => pending.searches.push(event),
        WriteOp::View(view) => pending.views.push(view),
        // A barrier must not be acknowledged before everything ahead of it is committed.
        WriteOp::Flush(ack) => {
            commit(conn, counters, pending, geo);
            let _ = ack.send(());
        }
        WriteOp::Ban(BanWrite::Insert(ban, ack)) => {
            // Commit the pending records first so the ban's ordering relative to them is
            // observable; an admin who bans an address can then read the rows that caused it.
            commit(conn, counters, pending, geo);
            let _ = ack.send(bans::insert(conn, &ban).map_err(|error| error.to_string()));
        }
        WriteOp::Ban(BanWrite::Delete(id, ack)) => {
            commit(conn, counters, pending, geo);
            let _ = ack.send(bans::delete(conn, id).map_err(|error| error.to_string()));
        }
        WriteOp::Ban(BanWrite::Purge(ack)) => {
            commit(conn, counters, pending, geo);
            let now = crate::clock::now_unix();
            let _ = ack.send(bans::purge_expired(conn, now).map_err(|error| error.to_string()));
        }
        WriteOp::Ban(BanWrite::Hit(id)) => {
            if let Err(error) = bans::record_hit(conn, id) {
                tracing::warn!(%error, id, "could not record a ban hit");
            }
        }
        // Rollups read the rows, write aggregates and delete the rows: all of it has to happen
        // on the writer, or it would race the very writes it is summarising.
        WriteOp::Maintenance(retention_days, now, ack) => {
            commit(conn, counters, pending, geo);
            let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                maintenance::maintain(conn, retention_days, now)
            }))
            .unwrap_or_default();
            let _ = ack.send(Ok(report));
        }
        WriteOp::PruneLists(keep, ack) => {
            commit(conn, counters, pending, geo);
            let _ = ack.send(lists::prune(conn, &keep).map_err(|error| error.to_string()));
        }
        WriteOp::PutList(id, payload, synergy, now, ack) => {
            commit(conn, counters, pending, geo);
            let _ = ack.send(
                lists::put(conn, &id, &payload, &synergy, now).map_err(|error| error.to_string()),
            );
        }
        WriteOp::Shutdown(ack) => {
            commit(conn, counters, pending, geo);
            checkpoint(conn);
            let _ = ack.send(());
            return false;
        }
    }

    true
}

fn commit(conn: &mut Connection, counters: &Counters, pending: &mut Pending, geo: Option<&Geo>) {
    if pending.is_empty() {
        return;
    }

    // Once per batch, not once per row: the writer holds no lock between batches, and a
    // memory-mapped lookup is cheap enough that batching it is about tidiness, not speed.
    if let Some(geo) = geo {
        for log in pending.requests.iter_mut() {
            if log.country.is_none() {
                if let Some(ip) = log.ip.as_deref().and_then(|ip| ip.parse().ok()) {
                    log.country = geo.country(ip);
                }
            }
        }
    }

    let result = request::insert(conn, &pending.requests)
        .and_then(|()| searches::insert(conn, &pending.searches))
        .and_then(|()| {
            for view in &pending.views {
                comparisons::record_view(conn, view)?;
            }
            Ok(())
        });

    match result {
        Ok(()) => {
            counters
                .rows
                .fetch_add(pending.len() as u64, Ordering::Relaxed);
            counters.commits.fetch_add(1, Ordering::Relaxed);
        }
        // Losing a batch must not kill the writer: the next batch may well succeed, and a
        // dead writer means no analytics for the rest of the process's life.
        Err(error) => tracing::error!(%error, rows = pending.len(), "could not write a batch"),
    }

    pending.clear();
}

/// Fold the WAL back into the main file so a backup or a file copy sees one consistent file,
/// and so the `-wal` sidecar does not grow without bound.
fn checkpoint(conn: &Connection) {
    if let Err(error) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;") {
        tracing::warn!(%error, "could not checkpoint the database on shutdown");
    }
}

/// A handful of read connections. The semaphore bounds concurrency; the `Vec` is a cache of
/// open connections, not a queue to wait on, so a waiter can never find it empty.
struct ReadPool {
    path: PathBuf,
    idle: Mutex<Vec<Connection>>,
    slots: Semaphore,
}

impl ReadPool {
    fn new(path: PathBuf, size: usize) -> Self {
        Self {
            path,
            idle: Mutex::new(Vec::with_capacity(size)),
            slots: Semaphore::new(size),
        }
    }

    async fn run<T, F>(&self, f: F) -> Result<T, AppError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let permit = self
            .slots
            .acquire()
            .await
            .map_err(|_| AppError::Store("the read pool is closed".into()))?;

        let idle = self
            .idle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop();
        let path = self.path.clone();

        // The connection is returned to the cache even when the query fails: a failed
        // statement does not invalidate the connection, and dropping it would mean reopening
        // on every bad query.
        let join =
            tokio::task::spawn_blocking(move || -> (Option<Connection>, Result<T, AppError>) {
                let conn = match idle {
                    Some(conn) => conn,
                    None => match open_connection(&path) {
                        Ok(conn) => conn,
                        Err(error) => return (None, Err(error)),
                    },
                };

                let result =
                    f(&conn).map_err(|error| AppError::Store(format!("query failed: {error}")));

                (Some(conn), result)
            });

        let (conn, result) = join
            .await
            .map_err(|error| AppError::Store(format!("read task failed: {error}")))?;

        if let Some(conn) = conn {
            self.idle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(conn);
        }
        drop(permit);

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::request::Kind;

    /// A database path unique to this test, cleaned up on drop.
    struct TempDb {
        dir: PathBuf,
    }

    impl TempDb {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("same-actor-store-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn config(&self) -> StoreConfig {
            StoreConfig {
                path: self.dir.join("test.db"),
                ..StoreConfig::default()
            }
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn log(path: &str) -> RequestLog {
        RequestLog {
            ts: 1_700_000_000,
            duration_ms: 3,
            ip: Some("203.0.113.7".into()),
            session: Some("sess-1".into()),
            ua: Some("Mozilla/5.0".into()),
            method: "GET".into(),
            path: path.into(),
            status: 200,
            code: None,
            message: None,
            kind: Kind::Api,
            country: Some("GB".into()),
        }
    }

    #[tokio::test]
    async fn a_record_is_written_and_readable() {
        let db = TempDb::new("roundtrip");
        let store = Store::open(db.config()).expect("open");

        assert!(store.record(log("/api/search")));
        store.flush().await.expect("flush");

        let rows = store.recent_requests(10).await.expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/api/search");
        assert_eq!(rows[0].status, 200);
        assert_eq!(rows[0].country.as_deref(), Some("GB"));

        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn the_writer_batches_a_burst_into_few_transactions() {
        let db = TempDb::new("batching");
        let store = Store::open(StoreConfig {
            batch_wait: Duration::from_millis(50),
            ..db.config()
        })
        .expect("open");

        for i in 0..500 {
            assert!(store.record(log(&format!("/api/search?i={i}"))));
        }
        store.flush().await.expect("flush");

        let stats = store.stats();
        assert_eq!(stats.rows, 500);
        assert!(
            stats.commits < 50,
            "500 rows took {} transactions; batching is not working",
            stats.commits
        );

        assert_eq!(store.recent_requests(1000).await.expect("read").len(), 500);
        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_flush_is_a_barrier_for_records_offered_before_it() {
        let db = TempDb::new("barrier");
        let store = Store::open(db.config()).expect("open");

        // No sleep, no polling: if the barrier were not ordered, this would flake.
        for i in 0..100 {
            store.record(log(&format!("/page/{i}")));
        }
        store.flush().await.expect("flush");

        let rows = store.recent_requests(1000).await.expect("read");
        assert_eq!(rows.len(), 100, "the barrier did not cover earlier records");
        store.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_full_or_closed_queue_drops_records_instead_of_blocking() {
        let db = TempDb::new("drop");
        let store = Store::open(StoreConfig {
            queue_capacity: 1,
            ..db.config()
        })
        .expect("open");

        // Shut the writer down first, so every subsequent offer is refused. This is the
        // deterministic version of "the queue is full": the behaviour under test is that a
        // refused record is counted and returned as false, never awaited.
        store.close().await.expect("close");

        let started = std::time::Instant::now();
        let mut accepted = 0;
        for _ in 0..1_000 {
            if store.record(log("/api/search")) {
                accepted += 1;
            }
        }
        let elapsed = started.elapsed();

        assert_eq!(accepted, 0, "a stopped writer must accept nothing");
        assert_eq!(store.stats().dropped, 1_000);
        assert!(
            elapsed < Duration::from_secs(1),
            "recording 1000 records against a stopped writer took {elapsed:?}; it is blocking"
        );
    }

    #[tokio::test]
    async fn wal_is_enabled_and_checkpointed_on_close() {
        let db = TempDb::new("wal");
        let store = Store::open(db.config()).expect("open");
        let path = db.dir.join("test.db");

        store.record(log("/api/search"));
        store.flush().await.expect("flush");
        assert!(
            path.with_extension("db-wal").exists(),
            "the write-ahead log should exist while the writer is live"
        );

        store.close().await.expect("close");

        let sidecar = path.with_extension("db-wal");
        let remaining = std::fs::metadata(&sidecar).map(|m| m.len()).unwrap_or(0);
        assert_eq!(remaining, 0, "the WAL was not truncated on shutdown");
    }

    #[tokio::test]
    async fn reopening_an_existing_database_keeps_its_rows() {
        let db = TempDb::new("reopen");
        let path = db.dir.join("test.db");

        {
            let store = Store::open(db.config()).expect("open");
            store.record(log("/page/one"));
            store.flush().await.expect("flush");
            store.close().await.expect("close");
        }

        let store = Store::open(StoreConfig {
            path,
            ..db.config()
        })
        .expect("reopen");
        assert_eq!(store.recent_requests(10).await.expect("read").len(), 1);
        store.close().await.expect("close");
    }
}
