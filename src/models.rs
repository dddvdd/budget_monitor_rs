use serde::{Deserialize, Serialize};

/// Convert centavos (i64) to pesos (f64) for API responses.
pub fn cents_to_pesos(cents: i64) -> f64 {
    (cents as f64) / 100.0
}

/// Convert pesos (f64) to centavos (i64) for exact storage and arithmetic.
pub fn pesos_to_cents(pesos: f64) -> i64 {
    (pesos * 100.0).round() as i64
}

/// Format centavos as Philippine Peso currency string, e.g. "PHP 1,234.56".
pub fn format_currency(cents: i64) -> String {
    let negative = cents < 0;
    let abs = cents.unsigned_abs();
    let whole = abs / 100;
    let frac = abs % 100;
    let whole_str = whole.to_string();
    let mut with_commas = String::with_capacity(whole_str.len() + whole_str.len() / 3);
    for (idx, ch) in whole_str.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            with_commas.push(',');
        }
        with_commas.push(ch);
    }
    let formatted_whole: String = with_commas.chars().rev().collect();
    if negative {
        format!("-PHP {}.{:02}", formatted_whole, frac)
    } else {
        format!("PHP {}.{:02}", formatted_whole, frac)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserRole {
    Admin,
    Encoder,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "Admin",
            UserRole::Encoder => "Encoder",
        }
    }

    pub fn from_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("Admin") {
            UserRole::Admin
        } else {
            UserRole::Encoder
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub user_id: i64,
    pub username: String,
    pub role: UserRole,
    pub full_name: String,
    pub is_active: bool,
    pub created_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartOfAccount {
    pub coa_id: i64,
    pub account_code: String,
    pub account_name: String,
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChartOfAccountRequest {
    pub account_code: String,
    pub account_name: String,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub account_id: i64,
    pub account_code: String,
    pub account_name: String,
    pub budget: f64,
    pub contingency: bool,
    pub base_available: f64,
    pub unobligated: f64,
    pub obligated: f64,
    pub available: f64,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub account_code: String,
    pub account_name: Option<String>,
    pub budget: f64,
    pub contingency: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAccountRequest {
    pub account_code: String,
    pub account_name: String,
    pub budget: f64,
    pub contingency: bool,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PRItemInput {
    pub unit: String,
    pub description: String,
    pub quantity: f64,
    pub unit_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PRItem {
    pub item_id: i64,
    pub pr_id: i64,
    pub line_number: i64,
    pub unit: String,
    pub description: String,
    pub quantity: f64,
    pub unit_cost: f64,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseRequest {
    pub pr_id: i64,
    pub pr_number: String,
    pub pr_date: String,
    pub department: String,
    pub fund_source: String,
    pub aip_code: Option<String>,
    pub account_code: String,
    pub payee: String,
    pub purpose: String,
    pub total_amount: f64,
    pub status: String,
    pub created_by: String,
    pub created_date: String,
    pub remarks: Option<String>,
    pub item_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<PRItem>>,
}

#[derive(Debug, Deserialize)]
pub struct SavePRRequest {
    pub pr_date: Option<String>,
    pub department: String,
    pub fund_source: String,
    pub aip_code: Option<String>,
    pub account_code: String,
    pub payee: String,
    pub purpose: String,
    pub status: Option<String>,
    pub remarks: Option<String>,
    pub items: Vec<PRItemInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationRequest {
    pub obr_id: i64,
    pub obr_number: String,
    pub obr_date: String,
    pub pr_id: Option<i64>,
    pub pr_number: Option<String>,
    pub account_code: String,
    pub payee: String,
    pub purpose: String,
    pub amount: f64,
    pub status: String,
    pub created_by: String,
    pub created_date: String,
}

#[derive(Debug, Deserialize)]
pub struct ConvertPRToOBRRequest {
    pub pr_id: i64,
    pub amount: f64,
    #[serde(default)]
    pub force_over_budget: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateDirectOBRRequest {
    pub account_code: String,
    pub payee: String,
    pub purpose: String,
    pub amount: f64,
    #[serde(default)]
    pub force_over_budget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Augmentation {
    pub augmentation_id: i64,
    pub source_account_code: String,
    pub destination_account_code: String,
    pub amount: f64,
    pub justification: String,
    pub prepared_by: String,
    pub augmentation_date: String,
    pub created_date: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAugmentationRequest {
    pub source_account_code: String,
    pub destination_account_code: String,
    pub amount: f64,
    pub justification: String,
    pub date: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "outcome")]
pub enum ConvertPRToOBRResponse {
    Created {
        obr: ObligationRequest,
    },
    BudgetWarning {
        requested: f64,
        available: f64,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisbursementVoucher {
    pub dv_id: i64,
    pub dv_number: String,
    pub dv_date: String,
    pub obr_id: i64,
    pub obr_number: String,
    pub payee: String,
    pub amount: f64,
    pub check_no: Option<String>,
    pub check_date: Option<String>,
    pub status: String,
    pub created_by: String,
    pub created_date: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateDVRequest {
    pub obr_id: i64,
    pub check_no: Option<String>,
    pub check_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub audit_id: i64,
    pub timestamp: String,
    pub username: String,
    pub action: String,
    pub table_name: String,
    pub record_id: i64,
    pub details: Option<String>,
    pub ip_address: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AuditFilter {
    pub username: Option<String>,
    pub action: Option<String>,
    pub table_name: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DashboardSummary {
    pub total_budget: f64,
    pub total_available: f64,
    pub total_unobligated: f64,
    pub total_obligated: f64,
    pub active_accounts: i64,
    pub exhausted_accounts: i64,
    pub pr_count: i64,
    pub obr_count: i64,
    pub dv_count: i64,
    pub updated_at: String,
    pub accounts: Vec<Account>,
    pub recent_activities: Vec<AuditEntry>,
}

