use chrono::Local;
use rusqlite::{params, types::Value, Connection, OptionalExtension};
use std::fmt;

use crate::db::next_sequence_number;
use crate::models::*;

#[derive(Debug)]
pub enum ServiceError {
    NotFound(String),
    Validation(String),
    Forbidden(String),
    Conflict(String),
    Database(rusqlite::Error),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServiceError::NotFound(m) => write!(f, "{}", m),
            ServiceError::Validation(m) => write!(f, "{}", m),
            ServiceError::Forbidden(m) => write!(f, "{}", m),
            ServiceError::Conflict(m) => write!(f, "{}", m),
            ServiceError::Database(e) => write!(f, "Database error: {}", e),
        }
    }
}

impl From<rusqlite::Error> for ServiceError {
    fn from(err: rusqlite::Error) -> Self {
        ServiceError::Database(err)
    }
}

pub type ServiceResult<T> = Result<T, ServiceError>;

fn now_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

fn today_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn compute_base_available(budget_cents: i64, contingency: bool) -> i64 {
    if contingency {
        (budget_cents as f64 * 0.9).round() as i64
    } else {
        budget_cents
    }
}

// ============================================================================
// AUTH & AUDIT
// ============================================================================

pub fn authenticate_user(
    conn: &Connection,
    username: &str,
    password: &str,
) -> ServiceResult<User> {
    let user = conn
        .query_row(
            "SELECT UserID, Username, Role, FullName, IsActive, CreatedDate
             FROM tblUsers
             WHERE Username = ?1 AND Password = ?2 AND IsActive = 1",
            params![username.trim(), password],
            |r| {
                let role_str: String = r.get(2)?;
                Ok(User {
                    user_id: r.get(0)?,
                    username: r.get(1)?,
                    role: UserRole::from_str(&role_str),
                    full_name: r.get(3)?,
                    is_active: r.get::<_, i64>(4)? != 0,
                    created_date: r.get(5)?,
                })
            },
        )
        .optional()?;

    user.ok_or_else(|| ServiceError::Validation("Invalid username or password".to_string()))
}

pub fn log_audit(
    conn: &Connection,
    username: &str,
    action: &str,
    table_name: &str,
    record_id: i64,
    details: Option<&str>,
    ip_address: Option<&str>,
) {
    let _ = conn.execute(
        "INSERT INTO tblAuditTrail (Timestamp, Username, Action, TableName, RecordID, Details, IPAddress)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            now_timestamp(),
            username,
            action,
            table_name,
            record_id,
            details,
            ip_address
        ],
    );
}

// ============================================================================
// CHART OF ACCOUNTS
// ============================================================================

pub fn list_chart_of_accounts(conn: &Connection) -> ServiceResult<Vec<ChartOfAccount>> {
    let mut stmt = conn.prepare(
        "SELECT COAID, AccountCode, AccountName, Category
         FROM tblChartOfAccounts
         ORDER BY AccountCode ASC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ChartOfAccount {
            coa_id: r.get(0)?,
            account_code: r.get(1)?,
            account_name: r.get(2)?,
            category: r.get(3)?,
        })
    })?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

pub fn create_chart_of_account(
    conn: &Connection,
    req: &CreateChartOfAccountRequest,
    username: &str,
) -> ServiceResult<ChartOfAccount> {
    let code = req.account_code.trim();
    let name = req.account_name.trim();
    if code.is_empty() || name.is_empty() {
        return Err(ServiceError::Validation(
            "Account code and account name are required".to_string(),
        ));
    }

    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblChartOfAccounts WHERE AccountCode = ?1",
        params![code],
        |r| r.get(0),
    )?;
    if exists > 0 {
        return Err(ServiceError::Conflict(format!(
            "Chart of Accounts code '{}' already exists",
            code
        )));
    }

    conn.execute(
        "INSERT INTO tblChartOfAccounts (AccountCode, AccountName, Category) VALUES (?1, ?2, ?3)",
        params![code, name, req.category.as_deref()],
    )?;
    let id = conn.last_insert_rowid();
    log_audit(
        conn,
        username,
        "COA Created",
        "tblChartOfAccounts",
        id,
        Some(&format!("Code: {} - {}", code, name)),
        None,
    );

    Ok(ChartOfAccount {
        coa_id: id,
        account_code: code.to_string(),
        account_name: name.to_string(),
        category: req.category.clone(),
    })
}

// ============================================================================
// BUDGET ACCOUNTS
// ============================================================================

fn row_to_account(r: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    let budget_cents: i64 = r.get(3)?;
    let contingency = r.get::<_, i64>(4)? != 0;
    let available_cents: i64 = r.get(5)?;
    let unobligated_cents: i64 = r.get(7)?;
    let obligated_cents: i64 = r.get(8)?;
    let base_cents = compute_base_available(budget_cents, contingency);

    Ok(Account {
        account_id: r.get(0)?,
        account_code: r.get(1)?,
        account_name: r.get(2)?,
        budget: cents_to_pesos(budget_cents),
        contingency,
        base_available: cents_to_pesos(base_cents),
        unobligated: cents_to_pesos(unobligated_cents),
        obligated: cents_to_pesos(obligated_cents),
        available: cents_to_pesos(available_cents),
        status: r.get(6)?,
    })
}

const ACCOUNT_SELECT: &str = "
    SELECT a.AccountID, a.AccountCode, a.AccountName, a.Budget, a.Contingency, a.Available, a.Status,
           COALESCE((SELECT SUM(p.TotalAmount) FROM tblPR p WHERE p.AccountCode = a.AccountCode AND p.Status NOT IN ('Served', 'Cancelled')), 0) AS Unobligated,
           COALESCE((SELECT SUM(o.Amount) FROM tblOBR o WHERE o.AccountCode = a.AccountCode), 0) AS Obligated
    FROM tblAccounts a
";

pub fn list_accounts(conn: &Connection) -> ServiceResult<Vec<Account>> {
    let sql = format!("{} ORDER BY a.AccountCode ASC", ACCOUNT_SELECT);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_account)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_account(conn: &Connection, account_id: i64) -> ServiceResult<Account> {
    let sql = format!("{} WHERE a.AccountID = ?1", ACCOUNT_SELECT);
    conn.query_row(&sql, params![account_id], row_to_account)
        .optional()?
        .ok_or_else(|| ServiceError::NotFound("Account not found".to_string()))
}

pub fn create_account(
    conn: &Connection,
    req: &CreateAccountRequest,
    username: &str,
) -> ServiceResult<Account> {
    let code = req.account_code.trim();
    if code.is_empty() {
        return Err(ServiceError::Validation("Account Code is required".to_string()));
    }
    if req.budget <= 0.0 {
        return Err(ServiceError::Validation(
            "Budget must be greater than zero".to_string(),
        ));
    }

    // Verify AccountCode exists in Chart of Accounts
    let coa_name: Option<String> = conn
        .query_row(
            "SELECT AccountName FROM tblChartOfAccounts WHERE AccountCode = ?1",
            params![code],
            |r| r.get(0),
        )
        .optional()?;
    let coa_name = coa_name.ok_or_else(|| {
        ServiceError::Validation(format!(
            "Account Code '{}' does not exist in Chart of Accounts",
            code
        ))
    })?;

    let existing: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblAccounts WHERE AccountCode = ?1",
        params![code],
        |r| r.get(0),
    )?;
    if existing > 0 {
        return Err(ServiceError::Conflict(format!(
            "Budget account for '{}' already exists",
            code
        )));
    }

    let account_name = req
        .account_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&coa_name);

    let budget_cents = pesos_to_cents(req.budget);
    let available_cents = compute_base_available(budget_cents, req.contingency);
    let status = if available_cents <= 0 {
        "Exhausted"
    } else {
        "Active"
    };

    conn.execute(
        "INSERT INTO tblAccounts (AccountCode, AccountName, Budget, Contingency, Available, Status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            code,
            account_name,
            budget_cents,
            if req.contingency { 1 } else { 0 },
            available_cents,
            status
        ],
    )?;

    let account_id = conn.last_insert_rowid();
    log_audit(
        conn,
        username,
        "Account Created",
        "tblAccounts",
        account_id,
        Some(&format!(
            "Code: {}, Budget: {}, Contingency: {}",
            code,
            format_currency(budget_cents),
            if req.contingency { "Yes" } else { "No" }
        )),
        None,
    );

    get_account(conn, account_id)
}

pub fn update_account(
    conn: &Connection,
    account_id: i64,
    req: &UpdateAccountRequest,
    username: &str,
) -> ServiceResult<Account> {
    let existing = get_account(conn, account_id)?;
    let code = req.account_code.trim();
    let name = req.account_name.trim();

    if code.is_empty() {
        return Err(ServiceError::Validation("Account Code is required".to_string()));
    }
    if name.is_empty() {
        return Err(ServiceError::Validation("Account Name is required".to_string()));
    }
    if req.budget <= 0.0 {
        return Err(ServiceError::Validation(
            "Budget must be greater than zero".to_string(),
        ));
    }

    if code != existing.account_code {
        let dup: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tblAccounts WHERE AccountCode = ?1 AND AccountID <> ?2",
            params![code, account_id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(ServiceError::Conflict(
                "Account Code already exists. Choose a different code.".to_string(),
            ));
        }
    }

    let budget_cents = pesos_to_cents(req.budget);
    let base_cents = compute_base_available(budget_cents, req.contingency);

    // Subtract existing OBRs for this account so Available stays accurate
    let total_obr_cents: i64 = conn.query_row(
        "SELECT COALESCE(SUM(Amount), 0) FROM tblOBR WHERE AccountCode = ?1",
        params![code],
        |r| r.get(0),
    )?;
    let available_cents = base_cents - total_obr_cents;
    let status = req
        .status
        .as_deref()
        .filter(|s| *s == "Active" || *s == "Exhausted")
        .unwrap_or(if available_cents <= 0 {
            "Exhausted"
        } else {
            "Active"
        });

    conn.execute(
        "UPDATE tblAccounts
         SET AccountCode = ?1, AccountName = ?2, Budget = ?3, Contingency = ?4, Available = ?5, Status = ?6
         WHERE AccountID = ?7",
        params![
            code,
            name,
            budget_cents,
            if req.contingency { 1 } else { 0 },
            available_cents,
            status,
            account_id
        ],
    )?;

    log_audit(
        conn,
        username,
        "Account Updated",
        "tblAccounts",
        account_id,
        Some(&format!(
            "Code: {}, Budget: {}, Contingency: {}",
            code,
            format_currency(budget_cents),
            if req.contingency { "Yes" } else { "No" }
        )),
        None,
    );

    get_account(conn, account_id)
}

pub fn delete_account(conn: &Connection, account_id: i64, username: &str) -> ServiceResult<()> {
    let existing = get_account(conn, account_id)?;

    let pr_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblPR WHERE AccountCode = ?1",
        params![existing.account_code],
        |r| r.get(0),
    )?;
    let obr_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblOBR WHERE AccountCode = ?1",
        params![existing.account_code],
        |r| r.get(0),
    )?;

    if pr_count > 0 || obr_count > 0 {
        return Err(ServiceError::Conflict(format!(
            "Cannot delete account '{}' because it is referenced by {} PR(s) and {} OBR(s)",
            existing.account_code, pr_count, obr_count
        )));
    }

    conn.execute(
        "DELETE FROM tblAccounts WHERE AccountID = ?1",
        params![account_id],
    )?;

    log_audit(
        conn,
        username,
        "Account Deleted",
        "tblAccounts",
        account_id,
        Some(&format!(
            "Code: {} - {}",
            existing.account_code, existing.account_name
        )),
        None,
    );

    Ok(())
}

/// Reconcile stored Available and Status for all accounts from tblOBR sums (SQL/03_maintenance.sql).
pub fn recalculate_all_available(conn: &Connection, username: &str) -> ServiceResult<usize> {
    let accounts = list_accounts(conn)?;
    let mut updated = 0usize;
    for acc in accounts {
        let budget_cents = pesos_to_cents(acc.budget);
        let base_cents = compute_base_available(budget_cents, acc.contingency);
        let obr_sum: i64 = conn.query_row(
            "SELECT COALESCE(SUM(Amount), 0) FROM tblOBR WHERE AccountCode = ?1",
            params![acc.account_code],
            |r| r.get(0),
        )?;
        let available_cents = base_cents - obr_sum;
        let status = if available_cents <= 0 {
            "Exhausted"
        } else {
            "Active"
        };
        conn.execute(
            "UPDATE tblAccounts SET Available = ?1, Status = ?2 WHERE AccountID = ?3",
            params![available_cents, status, acc.account_id],
        )?;
        updated += 1;
    }

    log_audit(
        conn,
        username,
        "Budgets Recalculated",
        "tblAccounts",
        0,
        Some(&format!("Recalculated {} accounts", updated)),
        None,
    );

    Ok(updated)
}

// ============================================================================
// PURCHASE REQUESTS (PR + ITEMS)
// ============================================================================

pub fn list_prs(conn: &Connection) -> ServiceResult<Vec<PurchaseRequest>> {
    let mut stmt = conn.prepare(
        "SELECT
            p.PRID, p.PRNumber, p.PRDate, p.Department, p.FundSource, p.AIPCode,
            p.AccountCode, p.Payee, p.Purpose, p.TotalAmount, p.Status,
            p.CreatedBy, p.CreatedDate, p.Remarks, COUNT(i.ItemID) AS ItemCount
         FROM tblPR p
         LEFT JOIN tblPRItems i ON p.PRID = i.PRID
         GROUP BY p.PRID
         ORDER BY p.PRID DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        let total_cents: i64 = r.get(9)?;
        Ok(PurchaseRequest {
            pr_id: r.get(0)?,
            pr_number: r.get(1)?,
            pr_date: r.get(2)?,
            department: r.get(3)?,
            fund_source: r.get(4)?,
            aip_code: r.get(5)?,
            account_code: r.get(6)?,
            payee: r.get(7)?,
            purpose: r.get(8)?,
            total_amount: cents_to_pesos(total_cents),
            status: r.get(10)?,
            created_by: r.get(11)?,
            created_date: r.get(12)?,
            remarks: r.get(13)?,
            item_count: r.get(14)?,
            items: None,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_pr(conn: &Connection, pr_id: i64) -> ServiceResult<PurchaseRequest> {
    let mut pr = conn
        .query_row(
            "SELECT
                PRID, PRNumber, PRDate, Department, FundSource, AIPCode,
                AccountCode, Payee, Purpose, TotalAmount, Status,
                CreatedBy, CreatedDate, Remarks
             FROM tblPR WHERE PRID = ?1",
            params![pr_id],
            |r| {
                let total_cents: i64 = r.get(9)?;
                Ok(PurchaseRequest {
                    pr_id: r.get(0)?,
                    pr_number: r.get(1)?,
                    pr_date: r.get(2)?,
                    department: r.get(3)?,
                    fund_source: r.get(4)?,
                    aip_code: r.get(5)?,
                    account_code: r.get(6)?,
                    payee: r.get(7)?,
                    purpose: r.get(8)?,
                    total_amount: cents_to_pesos(total_cents),
                    status: r.get(10)?,
                    created_by: r.get(11)?,
                    created_date: r.get(12)?,
                    remarks: r.get(13)?,
                    item_count: 0,
                    items: None,
                })
            },
        )
        .optional()?
        .ok_or_else(|| ServiceError::NotFound("Purchase Request not found".to_string()))?;

    let mut stmt = conn.prepare(
        "SELECT ItemID, PRID, LineNumber, Unit, Description, Quantity, UnitCost, TotalCost
         FROM tblPRItems WHERE PRID = ?1 ORDER BY LineNumber ASC",
    )?;
    let rows = stmt.query_map(params![pr_id], |r| {
        let unit_cost_cents: i64 = r.get(6)?;
        let total_cost_cents: i64 = r.get(7)?;
        Ok(PRItem {
            item_id: r.get(0)?,
            pr_id: r.get(1)?,
            line_number: r.get(2)?,
            unit: r.get(3)?,
            description: r.get(4)?,
            quantity: r.get(5)?,
            unit_cost: cents_to_pesos(unit_cost_cents),
            total_cost: cents_to_pesos(total_cost_cents),
        })
    })?;

    let mut items = Vec::new();
    for r in rows {
        items.push(r?);
    }
    pr.item_count = items.len() as i64;
    pr.items = Some(items);
    Ok(pr)
}

fn validate_pr_request(req: &SavePRRequest) -> ServiceResult<i64> {
    if req.department.trim().is_empty() {
        return Err(ServiceError::Validation(
            "Please enter Department/Office".to_string(),
        ));
    }
    if req.account_code.trim().is_empty() {
        return Err(ServiceError::Validation(
            "Please select Account Code".to_string(),
        ));
    }
    if req.payee.trim().is_empty() {
        return Err(ServiceError::Validation("Please enter Payee".to_string()));
    }
    if req.items.is_empty() {
        return Err(ServiceError::Validation(
            "Please add line items".to_string(),
        ));
    }

    let mut total_cents: i64 = 0;
    for (idx, item) in req.items.iter().enumerate() {
        if item.unit.trim().is_empty() || item.description.trim().is_empty() {
            return Err(ServiceError::Validation(format!(
                "Line item {} requires unit and description",
                idx + 1
            )));
        }
        if item.quantity <= 0.0 || item.unit_cost <= 0.0 {
            return Err(ServiceError::Validation(format!(
                "Line item {} must have positive quantity and unit cost",
                idx + 1
            )));
        }
        let unit_cost_cents = pesos_to_cents(item.unit_cost);
        let line_total = ((item.quantity * unit_cost_cents as f64).round()) as i64;
        total_cents += line_total;
    }

    if total_cents <= 0 {
        return Err(ServiceError::Validation(
            "Please add line items with valid amounts".to_string(),
        ));
    }
    Ok(total_cents)
}

pub fn create_pr(
    conn: &mut Connection,
    req: &SavePRRequest,
    username: &str,
) -> ServiceResult<PurchaseRequest> {
    let total_cents = validate_pr_request(req)?;
    let tx = conn.transaction()?;

    let pr_number = next_sequence_number(&tx, "PR")?;
    let pr_date = req
        .pr_date
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(today_date);
    let status = req
        .status
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Draft");
    let created_date = now_timestamp();

    tx.execute(
        "INSERT INTO tblPR (
            PRNumber, PRDate, Department, FundSource, AIPCode,
            AccountCode, Payee, Purpose, TotalAmount, Status,
            CreatedBy, CreatedDate, Remarks
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            pr_number,
            pr_date,
            req.department.trim(),
            req.fund_source.trim(),
            req.aip_code.as_deref().map(str::trim),
            req.account_code.trim(),
            req.payee.trim(),
            req.purpose.trim(),
            total_cents,
            status,
            username,
            created_date,
            req.remarks.as_deref().map(str::trim),
        ],
    )?;
    let pr_id = tx.last_insert_rowid();

    for (idx, item) in req.items.iter().enumerate() {
        let unit_cost_cents = pesos_to_cents(item.unit_cost);
        let line_total_cents = ((item.quantity * unit_cost_cents as f64).round()) as i64;
        tx.execute(
            "INSERT INTO tblPRItems (PRID, LineNumber, Unit, Description, Quantity, UnitCost, TotalCost)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                pr_id,
                (idx + 1) as i64,
                item.unit.trim(),
                item.description.trim(),
                item.quantity,
                unit_cost_cents,
                line_total_cents
            ],
        )?;
    }

    log_audit(
        &tx,
        username,
        "PR Created",
        "tblPR",
        pr_id,
        Some(&format!("PR: {}", pr_number)),
        None,
    );
    tx.commit()?;

    get_pr(conn, pr_id)
}

pub fn update_pr(
    conn: &mut Connection,
    pr_id: i64,
    req: &SavePRRequest,
    username: &str,
) -> ServiceResult<PurchaseRequest> {
    let existing = get_pr(conn, pr_id)?;
    let total_cents = validate_pr_request(req)?;
    let tx = conn.transaction()?;

    let pr_date = req
        .pr_date
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&existing.pr_date);
    let status = req
        .status
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&existing.status);

    tx.execute(
        "UPDATE tblPR SET
            PRDate = ?1, Department = ?2, FundSource = ?3, AIPCode = ?4,
            AccountCode = ?5, Payee = ?6, Purpose = ?7, TotalAmount = ?8,
            Status = ?9, Remarks = ?10
         WHERE PRID = ?11",
        params![
            pr_date,
            req.department.trim(),
            req.fund_source.trim(),
            req.aip_code.as_deref().map(str::trim),
            req.account_code.trim(),
            req.payee.trim(),
            req.purpose.trim(),
            total_cents,
            status,
            req.remarks.as_deref().map(str::trim),
            pr_id
        ],
    )?;

    tx.execute("DELETE FROM tblPRItems WHERE PRID = ?1", params![pr_id])?;
    for (idx, item) in req.items.iter().enumerate() {
        let unit_cost_cents = pesos_to_cents(item.unit_cost);
        let line_total_cents = ((item.quantity * unit_cost_cents as f64).round()) as i64;
        tx.execute(
            "INSERT INTO tblPRItems (PRID, LineNumber, Unit, Description, Quantity, UnitCost, TotalCost)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                pr_id,
                (idx + 1) as i64,
                item.unit.trim(),
                item.description.trim(),
                item.quantity,
                unit_cost_cents,
                line_total_cents
            ],
        )?;
    }

    log_audit(
        &tx,
        username,
        "PR Updated",
        "tblPR",
        pr_id,
        Some(&format!("PR: {}", existing.pr_number)),
        None,
    );
    tx.commit()?;

    get_pr(conn, pr_id)
}

pub fn delete_pr(conn: &Connection, pr_id: i64, username: &str) -> ServiceResult<()> {
    let existing = get_pr(conn, pr_id)?;
    let obr_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblOBR WHERE PRID = ?1",
        params![pr_id],
        |r| r.get(0),
    )?;
    if obr_count > 0 {
        return Err(ServiceError::Conflict(
            "Cannot delete PR that already has an Obligation Request (OBR)".to_string(),
        ));
    }

    conn.execute("DELETE FROM tblPR WHERE PRID = ?1", params![pr_id])?;
    log_audit(
        conn,
        username,
        "PR Deleted",
        "tblPR",
        pr_id,
        Some(&format!("PR: {}", existing.pr_number)),
        None,
    );
    Ok(())
}

// ============================================================================
// OBLIGATION REQUESTS (OBR)
// ============================================================================

fn row_to_obr(r: &rusqlite::Row<'_>) -> rusqlite::Result<ObligationRequest> {
    let amount_cents: i64 = r.get(8)?;
    Ok(ObligationRequest {
        obr_id: r.get(0)?,
        obr_number: r.get(1)?,
        obr_date: r.get(2)?,
        pr_id: r.get(3)?,
        pr_number: r.get(4)?,
        account_code: r.get(5)?,
        payee: r.get(6)?,
        purpose: r.get(7)?,
        amount: cents_to_pesos(amount_cents),
        status: r.get(9)?,
        created_by: r.get(10)?,
        created_date: r.get(11)?,
    })
}

pub fn list_obrs(conn: &Connection) -> ServiceResult<Vec<ObligationRequest>> {
    let mut stmt = conn.prepare(
        "SELECT OBRID, OBRNumber, OBRDate, PRID, PRNumber, AccountCode,
                Payee, Purpose, Amount, Status, CreatedBy, CreatedDate
         FROM tblOBR ORDER BY OBRID DESC",
    )?;
    let rows = stmt.query_map([], row_to_obr)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_obr(conn: &Connection, obr_id: i64) -> ServiceResult<ObligationRequest> {
    conn.query_row(
        "SELECT OBRID, OBRNumber, OBRDate, PRID, PRNumber, AccountCode,
                Payee, Purpose, Amount, Status, CreatedBy, CreatedDate
         FROM tblOBR WHERE OBRID = ?1",
        params![obr_id],
        row_to_obr,
    )
    .optional()?
    .ok_or_else(|| ServiceError::NotFound("Obligation Request not found".to_string()))
}

pub fn convert_pr_to_obr(
    conn: &mut Connection,
    req: &ConvertPRToOBRRequest,
    username: &str,
) -> ServiceResult<ConvertPRToOBRResponse> {
    if req.amount <= 0.0 {
        return Err(ServiceError::Validation(
            "OBR amount must be greater than zero".to_string(),
        ));
    }

    let pr = get_pr(conn, req.pr_id)?;
    let amount_cents = pesos_to_cents(req.amount);

    // Check available budget for the PR's AccountCode
    let available_cents: i64 = conn
        .query_row(
            "SELECT Available FROM tblAccounts WHERE AccountCode = ?1",
            params![pr.account_code],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);

    if amount_cents > available_cents && !req.force_over_budget {
        return Ok(ConvertPRToOBRResponse::BudgetWarning {
            requested: cents_to_pesos(amount_cents),
            available: cents_to_pesos(available_cents),
            message: format!(
                "Insufficient budget! Requested: {}, Available: {}. Continue anyway?",
                format_currency(amount_cents),
                format_currency(available_cents)
            ),
        });
    }

    let tx = conn.transaction()?;
    let obr_number = next_sequence_number(&tx, "OBR")?;
    let obr_date = today_date();
    let created_date = now_timestamp();

    tx.execute(
        "INSERT INTO tblOBR (
            OBRNumber, OBRDate, PRID, PRNumber, AccountCode,
            Payee, Purpose, Amount, Status, CreatedBy, CreatedDate
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'Approved', ?9, ?10)",
        params![
            obr_number,
            obr_date,
            pr.pr_id,
            pr.pr_number,
            pr.account_code,
            pr.payee,
            pr.purpose,
            amount_cents,
            username,
            created_date
        ],
    )?;
    let obr_id = tx.last_insert_rowid();

    // Side effects matching modOBR.bas
    tx.execute(
        "UPDATE tblPR SET Status = 'Served' WHERE PRID = ?1",
        params![pr.pr_id],
    )?;
    tx.execute(
        "UPDATE tblAccounts SET Available = Available - ?1 WHERE AccountCode = ?2",
        params![amount_cents, pr.account_code],
    )?;
    tx.execute(
        "UPDATE tblAccounts SET Status = 'Exhausted' WHERE Available <= 0 AND AccountCode = ?1",
        params![pr.account_code],
    )?;

    log_audit(
        &tx,
        username,
        "OBR Created",
        "tblOBR",
        obr_id,
        Some(&format!(
            "PR: {} Amount: {}",
            pr.pr_number,
            format_currency(amount_cents)
        )),
        None,
    );

    tx.commit()?;

    let obr = get_obr(conn, obr_id)?;
    Ok(ConvertPRToOBRResponse::Created { obr })
}

pub fn create_direct_obr(
    conn: &mut Connection,
    req: &CreateDirectOBRRequest,
    username: &str,
) -> ServiceResult<ConvertPRToOBRResponse> {
    let code = req.account_code.trim();
    let payee = req.payee.trim();
    let purpose = req.purpose.trim();

    if code.is_empty() || payee.is_empty() || purpose.is_empty() {
        return Err(ServiceError::Validation(
            "Account Code, Payee, and Purpose are required".to_string(),
        ));
    }
    if req.amount <= 0.0 {
        return Err(ServiceError::Validation(
            "OBR amount must be greater than zero".to_string(),
        ));
    }

    let available_cents: i64 = conn
        .query_row(
            "SELECT Available FROM tblAccounts WHERE AccountCode = ?1",
            params![code],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| ServiceError::NotFound(format!("Budget account '{}' not found", code)))?;

    let amount_cents = pesos_to_cents(req.amount);
    if amount_cents > available_cents && !req.force_over_budget {
        return Ok(ConvertPRToOBRResponse::BudgetWarning {
            requested: cents_to_pesos(amount_cents),
            available: cents_to_pesos(available_cents),
            message: format!(
                "Insufficient budget! Requested: {}, Available: {}. Continue anyway?",
                format_currency(amount_cents),
                format_currency(available_cents)
            ),
        });
    }

    let tx = conn.transaction()?;
    let obr_number = next_sequence_number(&tx, "OBR")?;
    let obr_date = today_date();
    let created_date = now_timestamp();

    tx.execute(
        "INSERT INTO tblOBR (
            OBRNumber, OBRDate, PRID, PRNumber, AccountCode,
            Payee, Purpose, Amount, Status, CreatedBy, CreatedDate
         ) VALUES (?1, ?2, NULL, NULL, ?3, ?4, ?5, ?6, 'Approved', ?7, ?8)",
        params![
            obr_number,
            obr_date,
            code,
            payee,
            purpose,
            amount_cents,
            username,
            created_date
        ],
    )?;
    let obr_id = tx.last_insert_rowid();

    tx.execute(
        "UPDATE tblAccounts SET Available = Available - ?1 WHERE AccountCode = ?2",
        params![amount_cents, code],
    )?;
    tx.execute(
        "UPDATE tblAccounts SET Status = 'Exhausted' WHERE Available <= 0 AND AccountCode = ?1",
        params![code],
    )?;

    log_audit(
        &tx,
        username,
        "OBR Created",
        "tblOBR",
        obr_id,
        Some(&format!(
            "Direct OBR {} ({}) Amount: {}",
            obr_number,
            code,
            format_currency(amount_cents)
        )),
        None,
    );

    tx.commit()?;
    let obr = get_obr(conn, obr_id)?;
    Ok(ConvertPRToOBRResponse::Created { obr })
}

// ============================================================================
// BUDGET AUGMENTATIONS (TRANSFERS)
// ============================================================================

pub fn list_augmentations(conn: &Connection) -> ServiceResult<Vec<Augmentation>> {
    let mut stmt = conn.prepare(
        "SELECT AugmentationID, SourceAccountCode, DestinationAccountCode, Amount,
                Justification, PreparedBy, AugmentationDate, CreatedDate
         FROM tblAugmentations
         ORDER BY AugmentationID DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        let amount_cents: i64 = r.get(3)?;
        Ok(Augmentation {
            augmentation_id: r.get(0)?,
            source_account_code: r.get(1)?,
            destination_account_code: r.get(2)?,
            amount: cents_to_pesos(amount_cents),
            justification: r.get(4)?,
            prepared_by: r.get(5)?,
            augmentation_date: r.get(6)?,
            created_date: r.get(7)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn create_augmentation(
    conn: &mut Connection,
    req: &CreateAugmentationRequest,
    username: &str,
) -> ServiceResult<Augmentation> {
    let src_code = req.source_account_code.trim();
    let dst_code = req.destination_account_code.trim();
    let justification = req.justification.trim();

    if src_code.is_empty() || dst_code.is_empty() {
        return Err(ServiceError::Validation(
            "Source and destination accounts are required".to_string(),
        ));
    }
    if src_code == dst_code {
        return Err(ServiceError::Validation(
            "Source and destination accounts must be different".to_string(),
        ));
    }
    if req.amount <= 0.0 {
        return Err(ServiceError::Validation(
            "Transfer amount must be greater than zero".to_string(),
        ));
    }
    if justification.is_empty() {
        return Err(ServiceError::Validation(
            "Justification is required for budget augmentation".to_string(),
        ));
    }

    let amount_cents = pesos_to_cents(req.amount);

    let (src_budget, src_avail): (i64, i64) = conn
        .query_row(
            "SELECT Budget, Available FROM tblAccounts WHERE AccountCode = ?1",
            params![src_code],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| ServiceError::NotFound(format!("Source account '{}' not found", src_code)))?;

    if amount_cents > src_avail {
        return Err(ServiceError::Validation(format!(
            "Insufficient balance in source account '{}': available {}, requested {}",
            src_code,
            format_currency(src_avail),
            format_currency(amount_cents)
        )));
    }

    let (dst_budget, dst_cont, dst_avail): (i64, bool, i64) = conn
        .query_row(
            "SELECT Budget, Contingency, Available FROM tblAccounts WHERE AccountCode = ?1",
            params![dst_code],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| {
            ServiceError::NotFound(format!("Destination account '{}' not found", dst_code))
        })?;

    let new_src_budget = (src_budget - amount_cents).max(1);
    let new_src_avail = src_avail - amount_cents;
    let new_src_status = if new_src_avail <= 0 {
        "Exhausted"
    } else {
        "Active"
    };

    let added_avail = compute_base_available(amount_cents, dst_cont);
    let new_dst_budget = dst_budget + amount_cents;
    let new_dst_avail = dst_avail + added_avail;
    let new_dst_status = if new_dst_avail <= 0 {
        "Exhausted"
    } else {
        "Active"
    };

    let aug_date = req
        .date
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(today_date);
    let created_date = now_timestamp();

    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE tblAccounts SET Budget = ?1, Available = ?2, Status = ?3 WHERE AccountCode = ?4",
        params![new_src_budget, new_src_avail, new_src_status, src_code],
    )?;
    tx.execute(
        "UPDATE tblAccounts SET Budget = ?1, Available = ?2, Status = ?3 WHERE AccountCode = ?4",
        params![new_dst_budget, new_dst_avail, new_dst_status, dst_code],
    )?;

    tx.execute(
        "INSERT INTO tblAugmentations (
            SourceAccountCode, DestinationAccountCode, Amount,
            Justification, PreparedBy, AugmentationDate, CreatedDate
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            src_code,
            dst_code,
            amount_cents,
            justification,
            username,
            aug_date,
            created_date
        ],
    )?;
    let aug_id = tx.last_insert_rowid();

    log_audit(
        &tx,
        username,
        "Augmentation Created",
        "tblAugmentations",
        aug_id,
        Some(&format!(
            "{} -> {}: {} ({})",
            src_code,
            dst_code,
            format_currency(amount_cents),
            justification
        )),
        None,
    );
    tx.commit()?;

    Ok(Augmentation {
        augmentation_id: aug_id,
        source_account_code: src_code.to_string(),
        destination_account_code: dst_code.to_string(),
        amount: cents_to_pesos(amount_cents),
        justification: justification.to_string(),
        prepared_by: username.to_string(),
        augmentation_date: aug_date,
        created_date,
    })
}

// ============================================================================
// DISBURSEMENT VOUCHERS (DV)
// ============================================================================

fn row_to_dv(r: &rusqlite::Row<'_>) -> rusqlite::Result<DisbursementVoucher> {
    let amount_cents: i64 = r.get(6)?;
    Ok(DisbursementVoucher {
        dv_id: r.get(0)?,
        dv_number: r.get(1)?,
        dv_date: r.get(2)?,
        obr_id: r.get(3)?,
        obr_number: r.get(4)?,
        payee: r.get(5)?,
        amount: cents_to_pesos(amount_cents),
        check_no: r.get(7)?,
        check_date: r.get(8)?,
        status: r.get(9)?,
        created_by: r.get(10)?,
        created_date: r.get(11)?,
    })
}

pub fn list_dvs(conn: &Connection) -> ServiceResult<Vec<DisbursementVoucher>> {
    let mut stmt = conn.prepare(
        "SELECT DVID, DVNumber, DVDate, OBRID, OBRNumber, Payee,
                Amount, CheckNo, CheckDate, Status, CreatedBy, CreatedDate
         FROM tblDV ORDER BY DVID DESC",
    )?;
    let rows = stmt.query_map([], row_to_dv)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_dv(conn: &Connection, dv_id: i64) -> ServiceResult<DisbursementVoucher> {
    conn.query_row(
        "SELECT DVID, DVNumber, DVDate, OBRID, OBRNumber, Payee,
                Amount, CheckNo, CheckDate, Status, CreatedBy, CreatedDate
         FROM tblDV WHERE DVID = ?1",
        params![dv_id],
        row_to_dv,
    )
    .optional()?
    .ok_or_else(|| ServiceError::NotFound("Disbursement Voucher not found".to_string()))
}

pub fn create_dv(
    conn: &mut Connection,
    req: &CreateDVRequest,
    username: &str,
) -> ServiceResult<DisbursementVoucher> {
    let obr = get_obr(conn, req.obr_id)?;
    let amount_cents = pesos_to_cents(obr.amount);
    if amount_cents <= 0 {
        return Err(ServiceError::Validation("Invalid OBR amount!".to_string()));
    }

    let tx = conn.transaction()?;
    let dv_number = next_sequence_number(&tx, "DV")?;
    let dv_date = today_date();
    let created_date = now_timestamp();

    tx.execute(
        "INSERT INTO tblDV (
            DVNumber, DVDate, OBRID, OBRNumber, Payee,
            Amount, CheckNo, CheckDate, Status, CreatedBy, CreatedDate
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'Approved', ?9, ?10)",
        params![
            dv_number,
            dv_date,
            obr.obr_id,
            obr.obr_number,
            obr.payee,
            amount_cents,
            req.check_no.as_deref().map(str::trim),
            req.check_date.as_deref().map(str::trim),
            username,
            created_date
        ],
    )?;
    let dv_id = tx.last_insert_rowid();

    tx.execute(
        "UPDATE tblOBR SET Status = 'Paid' WHERE OBRID = ?1",
        params![obr.obr_id],
    )?;

    log_audit(
        &tx,
        username,
        "DV Created",
        "tblDV",
        dv_id,
        Some(&format!(
            "OBR: {} Amount: {}",
            obr.obr_number,
            format_currency(amount_cents)
        )),
        None,
    );

    tx.commit()?;
    get_dv(conn, dv_id)
}

// ============================================================================
// DASHBOARD & AUDIT TRAIL
// ============================================================================

pub fn get_dashboard_summary(conn: &Connection) -> ServiceResult<DashboardSummary> {
    let accounts = list_accounts(conn)?;
    let (total_budget_cents, total_available_cents): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(Budget), 0), COALESCE(SUM(Available), 0) FROM tblAccounts",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let total_unobligated_cents: i64 = conn.query_row(
        "SELECT COALESCE(SUM(TotalAmount), 0) FROM tblPR WHERE Status NOT IN ('Served', 'Cancelled')",
        [],
        |r| r.get(0),
    )?;
    let total_obligated_cents: i64 =
        conn.query_row("SELECT COALESCE(SUM(Amount), 0) FROM tblOBR", [], |r| {
            r.get(0)
        })?;
    let active_accounts: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblAccounts WHERE Status = 'Active'",
        [],
        |r| r.get(0),
    )?;
    let exhausted_accounts: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tblAccounts WHERE Status = 'Exhausted'",
        [],
        |r| r.get(0),
    )?;
    let pr_count: i64 = conn.query_row("SELECT COUNT(*) FROM tblPR", [], |r| r.get(0))?;
    let obr_count: i64 = conn.query_row("SELECT COUNT(*) FROM tblOBR", [], |r| r.get(0))?;
    let dv_count: i64 = conn.query_row("SELECT COUNT(*) FROM tblDV", [], |r| r.get(0))?;

    let recent_activities = list_audit_trail(conn, &AuditFilter::default())?
        .into_iter()
        .take(8)
        .collect();

    Ok(DashboardSummary {
        total_budget: cents_to_pesos(total_budget_cents),
        total_available: cents_to_pesos(total_available_cents),
        total_unobligated: cents_to_pesos(total_unobligated_cents),
        total_obligated: cents_to_pesos(total_obligated_cents),
        active_accounts,
        exhausted_accounts,
        pr_count,
        obr_count,
        dv_count,
        updated_at: now_timestamp(),
        accounts,
        recent_activities,
    })
}

pub fn list_audit_trail(
    conn: &Connection,
    filter: &AuditFilter,
) -> ServiceResult<Vec<AuditEntry>> {
    let mut sql = String::from(
        "SELECT AuditID, Timestamp, Username, Action, TableName, RecordID, Details, IPAddress
         FROM tblAuditTrail WHERE 1=1",
    );
    let mut bound: Vec<Value> = Vec::new();

    if let Some(ref u) = filter.username {
        let u = u.trim();
        if !u.is_empty() {
            sql.push_str(" AND Username = ?");
            bound.push(Value::Text(u.to_string()));
        }
    }
    if let Some(ref a) = filter.action {
        let a = a.trim();
        if !a.is_empty() {
            sql.push_str(" AND Action = ?");
            bound.push(Value::Text(a.to_string()));
        }
    }
    if let Some(ref t) = filter.table_name {
        let t = t.trim();
        if !t.is_empty() {
            sql.push_str(" AND TableName = ?");
            bound.push(Value::Text(t.to_string()));
        }
    }
    if let Some(ref df) = filter.date_from {
        let df = df.trim();
        if !df.is_empty() {
            sql.push_str(" AND Timestamp >= ?");
            bound.push(Value::Text(format!("{} 00:00:00", df)));
        }
    }
    if let Some(ref dt) = filter.date_to {
        let dt = dt.trim();
        if !dt.is_empty() {
            sql.push_str(" AND Timestamp <= ?");
            bound.push(Value::Text(format!("{} 23:59:59", dt)));
        }
    }

    sql.push_str(" ORDER BY Timestamp DESC, AuditID DESC LIMIT 500");

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bound.iter()), |r| {
        Ok(AuditEntry {
            audit_id: r.get(0)?,
            timestamp: r.get(1)?,
            username: r.get(2)?,
            action: r.get(3)?,
            table_name: r.get(4)?,
            record_id: r.get(5)?,
            details: r.get(6)?,
            ip_address: r.get(7)?,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

