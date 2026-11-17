use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use rusqlite::Connection;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};
use uuid::Uuid;

use crate::{
    models::*,
    reports::{
        render_audit_csv, render_budget_summary_report, render_dv_report, render_obr_report,
        render_pr_report,
    },
    services::{self, ServiceError},
};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub sessions: Arc<RwLock<HashMap<String, User>>>,
    pub exports_dir: PathBuf,
}

pub struct ApiError(ServiceError);

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            ServiceError::NotFound(_) => StatusCode::NOT_FOUND,
            ServiceError::Validation(_) => StatusCode::BAD_REQUEST,
            ServiceError::Forbidden(_) => StatusCode::FORBIDDEN,
            ServiceError::Conflict(_) => StatusCode::CONFLICT,
            ServiceError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.0.to_string() });
        (status, Json(body)).into_response()
    }
}

#[derive(Deserialize, Default)]
pub struct TokenQuery {
    pub token: Option<String>,
}

fn extract_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(stripped) = auth.strip_prefix("Bearer ") {
            return Some(stripped.trim().to_string());
        }
    }
    query_token.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn require_user(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<User, ApiError> {
    let token = extract_token(headers, query_token)
        .ok_or_else(|| ApiError(ServiceError::Forbidden("Authentication required".to_string())))?;
    let sessions = state.sessions.read().unwrap();
    sessions
        .get(&token)
        .cloned()
        .ok_or_else(|| ApiError(ServiceError::Forbidden("Invalid or expired session".to_string())))
}

fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
    query_token: Option<&str>,
) -> Result<User, ApiError> {
    let user = require_user(state, headers, query_token)?;
    if user.role != UserRole::Admin {
        return Err(ApiError(ServiceError::Forbidden(
            "Admin role required for this action".to_string(),
        )));
    }
    Ok(user)
}

// ============================================================================
// AUTH HANDLERS
// ============================================================================

pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let conn = state.db.lock().unwrap();
    let user = services::authenticate_user(&conn, &req.username, &req.password)?;
    services::log_audit(&conn, &user.username, "Login", "tblUsers", user.user_id, None, None);

    let token = Uuid::new_v4().to_string();
    state.sessions.write().unwrap().insert(token.clone(), user.clone());
    Ok(Json(LoginResponse { token, user }))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Some(token) = extract_token(&headers, None) {
        if let Some(user) = state.sessions.write().unwrap().remove(&token) {
            let conn = state.db.lock().unwrap();
            services::log_audit(&conn, &user.username, "Logout", "tblUsers", user.user_id, None, None);
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<User>, ApiError> {
    let user = require_user(&state, &headers, None)?;
    Ok(Json(user))
}

// ============================================================================
// DASHBOARD & CHART OF ACCOUNTS
// ============================================================================

pub async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DashboardSummary>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::get_dashboard_summary(&conn)?))
}

pub async fn list_coa(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ChartOfAccount>>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_chart_of_accounts(&conn)?))
}

pub async fn create_coa(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateChartOfAccountRequest>,
) -> Result<(StatusCode, Json<ChartOfAccount>), ApiError> {
    let user = require_admin(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    let coa = services::create_chart_of_account(&conn, &req, &user.username)?;
    Ok((StatusCode::CREATED, Json(coa)))
}

// ============================================================================
// BUDGET ACCOUNTS
// ============================================================================

pub async fn list_accounts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Account>>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_accounts(&conn)?))
}

pub async fn create_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAccountRequest>,
) -> Result<(StatusCode, Json<Account>), ApiError> {
    let user = require_admin(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    let acc = services::create_account(&conn, &req, &user.username)?;
    Ok((StatusCode::CREATED, Json(acc)))
}

pub async fn update_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAccountRequest>,
) -> Result<Json<Account>, ApiError> {
    let user = require_admin(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::update_account(&conn, id, &req, &user.username)?))
}

pub async fn delete_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_admin(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    services::delete_account(&conn, id, &user.username)?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

pub async fn recalculate_budgets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_admin(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    let count = services::recalculate_all_available(&conn, &user.username)?;
    Ok(Json(serde_json::json!({ "recalculated": count })))
}

// ============================================================================
// PURCHASE REQUESTS
// ============================================================================

pub async fn list_prs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<PurchaseRequest>>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_prs(&conn)?))
}

pub async fn get_pr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<PurchaseRequest>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::get_pr(&conn, id)?))
}

pub async fn create_pr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SavePRRequest>,
) -> Result<(StatusCode, Json<PurchaseRequest>), ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut conn = state.db.lock().unwrap();
    let pr = services::create_pr(&mut conn, &req, &user.username)?;
    Ok((StatusCode::CREATED, Json(pr)))
}

pub async fn update_pr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(req): Json<SavePRRequest>,
) -> Result<Json<PurchaseRequest>, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut conn = state.db.lock().unwrap();
    Ok(Json(services::update_pr(&mut conn, id, &req, &user.username)?))
}

pub async fn delete_pr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    services::delete_pr(&conn, id, &user.username)?;
    Ok(Json(serde_json::json!({ "deleted": true })))
}

// ============================================================================
// OBLIGATION REQUESTS (OBR) & DISBURSEMENT VOUCHERS (DV)
// ============================================================================

pub async fn list_obrs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ObligationRequest>>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_obrs(&conn)?))
}

pub async fn convert_pr_to_obr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ConvertPRToOBRRequest>,
) -> Result<Json<ConvertPRToOBRResponse>, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut conn = state.db.lock().unwrap();
    Ok(Json(services::convert_pr_to_obr(&mut conn, &req, &user.username)?))
}

pub async fn create_direct_obr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateDirectOBRRequest>,
) -> Result<Json<ConvertPRToOBRResponse>, ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut conn = state.db.lock().unwrap();
    Ok(Json(services::create_direct_obr(&mut conn, &req, &user.username)?))
}

pub async fn list_augmentations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Augmentation>>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_augmentations(&conn)?))
}

pub async fn create_augmentation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateAugmentationRequest>,
) -> Result<(StatusCode, Json<Augmentation>), ApiError> {
    let user = require_admin(&state, &headers, None)?;
    let mut conn = state.db.lock().unwrap();
    let aug = services::create_augmentation(&mut conn, &req, &user.username)?;
    Ok((StatusCode::CREATED, Json(aug)))
}

pub async fn list_dvs(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DisbursementVoucher>>, ApiError> {
    require_user(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_dvs(&conn)?))
}

pub async fn create_dv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateDVRequest>,
) -> Result<(StatusCode, Json<DisbursementVoucher>), ApiError> {
    let user = require_user(&state, &headers, None)?;
    let mut conn = state.db.lock().unwrap();
    let dv = services::create_dv(&mut conn, &req, &user.username)?;
    Ok((StatusCode::CREATED, Json(dv)))
}

// ============================================================================
// AUDIT TRAIL & REPORTS
// ============================================================================

pub async fn list_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<AuditFilter>,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
    require_admin(&state, &headers, None)?;
    let conn = state.db.lock().unwrap();
    Ok(Json(services::list_audit_trail(&conn, &filter)?))
}

pub async fn export_audit_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(filter): Query<AuditFilter>,
    Query(tq): Query<TokenQuery>,
) -> Result<Response, ApiError> {
    require_admin(&state, &headers, tq.token.as_deref())?;
    let conn = state.db.lock().unwrap();
    let entries = services::list_audit_trail(&conn, &filter)?;
    let csv = render_audit_csv(&entries);

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"AuditTrail.csv\"",
            ),
        ],
        csv,
    )
        .into_response())
}

fn save_report_file(exports_dir: &PathBuf, filename: &str, html: &str) {
    let _ = std::fs::create_dir_all(exports_dir);
    let path = exports_dir.join(filename);
    let _ = std::fs::write(path, html);
}

pub async fn report_pr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(tq): Query<TokenQuery>,
) -> Result<Html<String>, ApiError> {
    require_user(&state, &headers, tq.token.as_deref())?;
    let conn = state.db.lock().unwrap();
    let pr = services::get_pr(&conn, id)?;
    let html = render_pr_report(&pr);
    save_report_file(&state.exports_dir, &format!("{}.html", pr.pr_number), &html);
    Ok(Html(html))
}

pub async fn report_obr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(tq): Query<TokenQuery>,
) -> Result<Html<String>, ApiError> {
    require_user(&state, &headers, tq.token.as_deref())?;
    let conn = state.db.lock().unwrap();
    let obr = services::get_obr(&conn, id)?;
    let html = render_obr_report(&obr);
    save_report_file(&state.exports_dir, &format!("{}.html", obr.obr_number), &html);
    Ok(Html(html))
}

pub async fn report_dv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(tq): Query<TokenQuery>,
) -> Result<Html<String>, ApiError> {
    require_user(&state, &headers, tq.token.as_deref())?;
    let conn = state.db.lock().unwrap();
    let dv = services::get_dv(&conn, id)?;
    let obr = services::get_obr(&conn, dv.obr_id)?;
    let html = render_dv_report(&dv, &obr.account_code);
    save_report_file(&state.exports_dir, &format!("{}.html", dv.dv_number), &html);
    Ok(Html(html))
}

pub async fn report_budget_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(tq): Query<TokenQuery>,
) -> Result<Html<String>, ApiError> {
    require_user(&state, &headers, tq.token.as_deref())?;
    let conn = state.db.lock().unwrap();
    let summary = services::get_dashboard_summary(&conn)?;
    let html = render_budget_summary_report(&summary);
    save_report_file(&state.exports_dir, "Budget_Summary.html", &html);
    Ok(Html(html))
}

