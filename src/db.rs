use chrono::Local;
use rusqlite::{params, Connection, Result};
use std::path::Path;

const COC_CSV: &str = include_str!("../data/coc.csv");

/// Initialize a file-backed SQLite database with WAL mode and foreign keys.
pub fn open_database<P: AsRef<Path>>(path: P) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure_and_migrate(&conn)?;
    Ok(conn)
}

/// Initialize an in-memory SQLite database (primarily for tests).
#[cfg(test)]
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure_and_migrate(&conn)?;
    Ok(conn)
}

fn configure_and_migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        PRAGMA busy_timeout = 5000;
        ",
    )?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS tblUsers (
            UserID      INTEGER PRIMARY KEY AUTOINCREMENT,
            Username    TEXT NOT NULL UNIQUE,
            Password    TEXT NOT NULL,
            Role        TEXT NOT NULL CHECK (Role IN ('Admin', 'Encoder')),
            FullName    TEXT NOT NULL,
            IsActive    INTEGER NOT NULL DEFAULT 1,
            CreatedDate TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );

        CREATE TABLE IF NOT EXISTS tblChartOfAccounts (
            COAID       INTEGER PRIMARY KEY AUTOINCREMENT,
            AccountCode TEXT NOT NULL UNIQUE,
            AccountName TEXT NOT NULL,
            Category    TEXT
        );

        CREATE TABLE IF NOT EXISTS tblAccounts (
            AccountID   INTEGER PRIMARY KEY AUTOINCREMENT,
            AccountCode TEXT NOT NULL UNIQUE,
            AccountName TEXT NOT NULL,
            Budget      INTEGER NOT NULL CHECK (Budget > 0),
            Contingency INTEGER NOT NULL DEFAULT 0,
            Available   INTEGER NOT NULL,
            Status      TEXT NOT NULL DEFAULT 'Active' CHECK (Status IN ('Active', 'Exhausted')),
            FOREIGN KEY (AccountCode) REFERENCES tblChartOfAccounts(AccountCode)
                ON UPDATE CASCADE ON DELETE RESTRICT
        );

        CREATE TABLE IF NOT EXISTS tblPR (
            PRID        INTEGER PRIMARY KEY AUTOINCREMENT,
            PRNumber    TEXT NOT NULL UNIQUE,
            PRDate      TEXT NOT NULL,
            Department  TEXT NOT NULL,
            FundSource  TEXT NOT NULL,
            AIPCode     TEXT,
            AccountCode TEXT NOT NULL,
            Payee       TEXT NOT NULL,
            Purpose     TEXT NOT NULL,
            TotalAmount INTEGER NOT NULL,
            Status      TEXT NOT NULL DEFAULT 'Draft',
            CreatedBy   TEXT NOT NULL,
            CreatedDate TEXT NOT NULL,
            Remarks     TEXT
        );

        CREATE TABLE IF NOT EXISTS tblPRItems (
            ItemID      INTEGER PRIMARY KEY AUTOINCREMENT,
            PRID        INTEGER NOT NULL,
            LineNumber  INTEGER NOT NULL,
            Unit        TEXT NOT NULL,
            Description TEXT NOT NULL,
            Quantity    REAL NOT NULL CHECK (Quantity > 0),
            UnitCost    INTEGER NOT NULL CHECK (UnitCost > 0),
            TotalCost   INTEGER NOT NULL,
            FOREIGN KEY (PRID) REFERENCES tblPR(PRID) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS tblOBR (
            OBRID       INTEGER PRIMARY KEY AUTOINCREMENT,
            OBRNumber   TEXT NOT NULL UNIQUE,
            OBRDate     TEXT NOT NULL,
            PRID        INTEGER NULL,
            PRNumber    TEXT NULL,
            AccountCode TEXT NOT NULL,
            Payee       TEXT NOT NULL,
            Purpose     TEXT NOT NULL,
            Amount      INTEGER NOT NULL CHECK (Amount > 0),
            Status      TEXT NOT NULL DEFAULT 'Approved',
            CreatedBy   TEXT NOT NULL,
            CreatedDate TEXT NOT NULL,
            FOREIGN KEY (PRID) REFERENCES tblPR(PRID) ON DELETE RESTRICT
        );

        CREATE TABLE IF NOT EXISTS tblAugmentations (
            AugmentationID         INTEGER PRIMARY KEY AUTOINCREMENT,
            SourceAccountCode      TEXT NOT NULL,
            DestinationAccountCode TEXT NOT NULL,
            Amount                 INTEGER NOT NULL CHECK (Amount > 0),
            Justification          TEXT NOT NULL,
            PreparedBy             TEXT NOT NULL,
            AugmentationDate       TEXT NOT NULL,
            CreatedDate            TEXT NOT NULL,
            FOREIGN KEY (SourceAccountCode) REFERENCES tblAccounts(AccountCode) ON UPDATE CASCADE ON DELETE RESTRICT,
            FOREIGN KEY (DestinationAccountCode) REFERENCES tblAccounts(AccountCode) ON UPDATE CASCADE ON DELETE RESTRICT
        );

        CREATE TABLE IF NOT EXISTS tblDV (
            DVID        INTEGER PRIMARY KEY AUTOINCREMENT,
            DVNumber    TEXT NOT NULL UNIQUE,
            DVDate      TEXT NOT NULL,
            OBRID       INTEGER NOT NULL,
            OBRNumber   TEXT NOT NULL,
            Payee       TEXT NOT NULL,
            Amount      INTEGER NOT NULL CHECK (Amount > 0),
            CheckNo     TEXT,
            CheckDate   TEXT,
            Status      TEXT NOT NULL DEFAULT 'Approved',
            CreatedBy   TEXT NOT NULL,
            CreatedDate TEXT NOT NULL,
            FOREIGN KEY (OBRID) REFERENCES tblOBR(OBRID) ON DELETE RESTRICT
        );

        CREATE TABLE IF NOT EXISTS tblAuditTrail (
            AuditID     INTEGER PRIMARY KEY AUTOINCREMENT,
            Timestamp   TEXT NOT NULL,
            Username    TEXT NOT NULL,
            Action      TEXT NOT NULL,
            TableName   TEXT NOT NULL,
            RecordID    INTEGER NOT NULL,
            Details     TEXT,
            IPAddress   TEXT
        );

        CREATE TABLE IF NOT EXISTS tblSequence (
            SeqName      TEXT PRIMARY KEY,
            Prefix       TEXT NOT NULL,
            CurrentValue INTEGER NOT NULL DEFAULT 0,
            LastUpdated  TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_tblAccounts_Code ON tblAccounts(AccountCode);
        CREATE INDEX IF NOT EXISTS idx_tblPR_AccountCode ON tblPR(AccountCode);
        CREATE INDEX IF NOT EXISTS idx_tblPR_Status ON tblPR(Status);
        CREATE INDEX IF NOT EXISTS idx_tblPRItems_PRID ON tblPRItems(PRID);
        CREATE INDEX IF NOT EXISTS idx_tblOBR_PRID ON tblOBR(PRID);
        CREATE INDEX IF NOT EXISTS idx_tblOBR_AccountCode ON tblOBR(AccountCode);
        CREATE INDEX IF NOT EXISTS idx_tblDV_OBRID ON tblDV(OBRID);
        CREATE INDEX IF NOT EXISTS idx_tblAuditTrail_Timestamp ON tblAuditTrail(Timestamp DESC);
        ",
    )?;

    seed_initial_data(conn)?;
    Ok(())
}

fn seed_initial_data(conn: &Connection) -> Result<()> {
    let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // Seed default users (matches SQL/01_tables.sql)
    conn.execute(
        "INSERT OR IGNORE INTO tblUsers (Username, Password, Role, FullName, IsActive, CreatedDate)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        params!["admin", "admin123", "Admin", "System Administrator", now],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO tblUsers (Username, Password, Role, FullName, IsActive, CreatedDate)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        params!["encoder", "encoder123", "Encoder", "Data Encoder", now],
    )?;

    // Seed sequences (matches SQL/04_sequences.sql)
    for (seq_name, prefix) in [("PR", "1011-"), ("OBR", "OBR-"), ("DV", "DV-")] {
        conn.execute(
            "INSERT OR IGNORE INTO tblSequence (SeqName, Prefix, CurrentValue, LastUpdated)
             VALUES (?1, ?2, 0, ?3)",
            params![seq_name, prefix, now],
        )?;
    }

    // Seed Chart of Accounts from coc.csv on disk if present, else embedded data/coc.csv
    let disk_csv = ["coc.csv", "data/coc.csv", "../coc.csv"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok());
    let csv_source = disk_csv.as_deref().unwrap_or(COC_CSV);
    seed_coa_from_csv(conn, csv_source)?;

    Ok(())
}

/// Parse a single CSV line respecting RFC-4180 double-quoted fields.
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes && chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = !in_quotes;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current.trim().to_string());
    fields
}

fn infer_coa_category(code: &str) -> &'static str {
    if code.starts_with("5-01") {
        "PS"
    } else if code.starts_with("5-02") {
        "MOOE"
    } else if code.starts_with("5-03") {
        "Financial Expenses"
    } else if code.starts_with("5-04") {
        "Non-Cash Expenses"
    } else if code.starts_with("1-07") || code.starts_with("1-08") || code.starts_with("1-09") {
        "Capital Outlay"
    } else if code.starts_with("1-") {
        "Assets"
    } else if code.starts_with("2-") {
        "Liabilities"
    } else if code.starts_with("3-") {
        "Equity"
    } else if code.starts_with("4-") {
        "Income"
    } else {
        "Other"
    }
}

pub fn seed_coa_from_csv(conn: &Connection, csv_content: &str) -> Result<usize> {
    let mut stmt = conn.prepare(
        "INSERT INTO tblChartOfAccounts (AccountCode, AccountName, Category)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(AccountCode) DO UPDATE SET
             AccountName = excluded.AccountName,
             Category = excluded.Category",
    )?;
    let mut inserted = 0usize;
    for (idx, raw_line) in csv_content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let cols = parse_csv_line(line);
        if cols.len() < 2 {
            continue;
        }
        let name = cols[0].trim();
        let code = cols[1].trim();
        if idx == 0 && name.eq_ignore_ascii_case("Account Name") {
            continue;
        }
        if name.is_empty() || code.is_empty() {
            continue;
        }
        let category = infer_coa_category(code);
        inserted += stmt.execute(params![code, name, category])?;
    }
    Ok(inserted)
}

/// Seed sample budget accounts and a sample PR for local development (`--dev`).
/// Idempotent: does nothing if `tblAccounts` already has rows.
pub fn seed_dev_data(conn: &Connection) -> Result<()> {
    let acc_count: i64 = conn.query_row("SELECT COUNT(*) FROM tblAccounts", [], |r| r.get(0))?;
    if acc_count > 0 {
        return Ok(());
    }

    let dev_accounts = [
        ("5-02-03-010", "Office Supplies Expenses", 500_000_00i64, 1i64, 450_000_00i64),
        ("5-02-01-010", "Traveling Expenses - Local", 200_000_00i64, 0i64, 200_000_00i64),
        ("5-02-02-010", "Training Expenses", 350_000_00i64, 1i64, 315_000_00i64),
    ];

    for (code, name, budget, cont, avail) in dev_accounts {
        conn.execute(
            "INSERT INTO tblAccounts (AccountCode, AccountName, Budget, Contingency, Available, Status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Active')",
            params![code, name, budget, cont, avail],
        )?;
    }

    Ok(())
}

/// Generate a collision-resistant sequence number in format `{Prefix}{YY}-{MM}-{NNN}`.
/// Must be called inside an active transaction or with exclusive connection lock.
pub fn next_sequence_number(conn: &Connection, seq_name: &str) -> Result<String> {
    let now = Local::now();
    let yy = now.format("%y").to_string();
    let mm = now.format("%m").to_string();
    let ts = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let (prefix, mut current_val): (String, i64) = conn.query_row(
        "SELECT Prefix, CurrentValue FROM tblSequence WHERE SeqName = ?1",
        params![seq_name],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let (table, col) = match seq_name {
        "PR" => ("tblPR", "PRNumber"),
        "OBR" => ("tblOBR", "OBRNumber"),
        "DV" => ("tblDV", "DVNumber"),
        _ => return Err(rusqlite::Error::InvalidQuery),
    };

    let mut candidate;
    let mut retries = 0;
    loop {
        current_val += 1;
        candidate = format!("{}{}-{}-{:03}", prefix, yy, mm, current_val);

        let check_sql = format!("SELECT COUNT(*) FROM {} WHERE {} = ?1", table, col);
        let exists: i64 = conn.query_row(&check_sql, params![candidate], |r| r.get(0))?;
        if exists == 0 {
            break;
        }
        retries += 1;
        if retries > 100 {
            candidate = format!("ERR-{}", now.format("%y%m%d-%H%M%S"));
            break;
        }
    }

    conn.execute(
        "UPDATE tblSequence SET CurrentValue = ?1, LastUpdated = ?2 WHERE SeqName = ?3",
        params![current_val, ts, seq_name],
    )?;

    Ok(candidate)
}

