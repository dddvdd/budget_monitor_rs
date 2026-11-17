mod db;
mod handlers;
mod models;
mod reports;
mod services;

use axum::{
    response::Html,
    routing::{get, post, put},
    Router,
};
use handlers::AppState;
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
};

const INDEX_HTML: &str = include_str!("../static/index.html");

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/logout", post(handlers::logout))
        .route("/api/auth/me", get(handlers::me))
        .route("/api/dashboard", get(handlers::dashboard))
        .route("/api/coa", get(handlers::list_coa).post(handlers::create_coa))
        .route(
            "/api/accounts",
            get(handlers::list_accounts).post(handlers::create_account),
        )
        .route(
            "/api/accounts/:id",
            put(handlers::update_account).delete(handlers::delete_account),
        )
        .route(
            "/api/accounts/recalculate",
            post(handlers::recalculate_budgets),
        )
        .route("/api/prs", get(handlers::list_prs).post(handlers::create_pr))
        .route(
            "/api/prs/:id",
            get(handlers::get_pr)
                .put(handlers::update_pr)
                .delete(handlers::delete_pr),
        )
        .route("/api/obrs", get(handlers::list_obrs))
        .route("/api/obrs/convert", post(handlers::convert_pr_to_obr))
        .route("/api/obrs/direct", post(handlers::create_direct_obr))
        .route(
            "/api/augmentations",
            get(handlers::list_augmentations).post(handlers::create_augmentation),
        )
        .route("/api/dvs", get(handlers::list_dvs).post(handlers::create_dv))
        .route("/api/audit", get(handlers::list_audit))
        .route("/api/audit/export", get(handlers::export_audit_csv))
        .route("/api/reports/pr/:id", get(handlers::report_pr))
        .route("/api/reports/obr/:id", get(handlers::report_obr))
        .route("/api/reports/dv/:id", get(handlers::report_dv))
        .route(
            "/api/reports/budget-summary",
            get(handlers::report_budget_summary),
        )
        .with_state(state)
}

#[tokio::main]
async fn main() {
    let mut bind_addr = String::from("127.0.0.1:8080");
    let mut db_path = PathBuf::from("budget_monitor.db");
    let mut exports_dir = if std::path::Path::new("../Exports/Reports").exists() {
        PathBuf::from("../Exports/Reports")
    } else {
        PathBuf::from("exports/reports")
    };
    let mut dev_mode = false;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" if i + 1 < args.len() => {
                bind_addr = args[i + 1].clone();
                i += 2;
            }
            "--db" if i + 1 < args.len() => {
                db_path = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--exports" if i + 1 < args.len() => {
                exports_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "--dev" => {
                dev_mode = true;
                i += 1;
            }
            _ => i += 1,
        }
    }

    let conn = db::open_database(&db_path).expect("Failed to open SQLite database");
    if dev_mode {
        db::seed_dev_data(&conn).expect("Failed to seed dev data");
        println!("Dev mode enabled: sample budget accounts seeded if empty");
    }
    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        sessions: Arc::new(RwLock::new(HashMap::new())),
        exports_dir,
    };

    let app = build_router(state);
    let addr: SocketAddr = bind_addr.parse().expect("Invalid bind address");
    println!("Budget Monitoring System running at http://{}", addr);
    println!("Database: {}", db_path.display());

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind TCP listener");
    axum::serve(listener, app).await.expect("Server error");
}

#[cfg(test)]
mod tests {
    use super::*;
    use models::*;

    #[test]
    fn end_to_end_budget_workflow() {
        let mut conn = db::open_in_memory().expect("in-memory db");

        // 1. Login validation
        let admin = services::authenticate_user(&conn, "admin", "admin123").unwrap();
        assert_eq!(admin.role, UserRole::Admin);
        let encoder = services::authenticate_user(&conn, "encoder", "encoder123").unwrap();
        assert_eq!(encoder.role, UserRole::Encoder);
        assert!(services::authenticate_user(&conn, "admin", "wrong").is_err());

        // 2. Create Budget Account with 10% contingency
        let acc = services::create_account(
            &conn,
            &CreateAccountRequest {
                account_code: "5-02-03-010".to_string(),
                account_name: None,
                budget: 100_000.0,
                contingency: true,
            },
            &admin.username,
        )
        .unwrap();
        assert_eq!(acc.base_available, 90_000.0);
        assert_eq!(acc.available, 90_000.0);
        assert_eq!(acc.status, "Active");

        // 3. Create PR with line items
        let pr = services::create_pr(
            &mut conn,
            &SavePRRequest {
                pr_date: None,
                department: "PESO".to_string(),
                fund_source: "General Fund".to_string(),
                aip_code: Some("AIP-2026-01".to_string()),
                account_code: "5-02-03-010".to_string(),
                payee: "ABC Supplies".to_string(),
                purpose: "Office supplies for Q4".to_string(),
                status: Some("Approved".to_string()),
                remarks: None,
                items: vec![
                    PRItemInput {
                        unit: "ream".to_string(),
                        description: "A4 Bond Paper".to_string(),
                        quantity: 50.0,
                        unit_cost: 250.0,
                    },
                    PRItemInput {
                        unit: "box".to_string(),
                        description: "Ballpen".to_string(),
                        quantity: 10.0,
                        unit_cost: 150.0,
                    },
                ],
            },
            &encoder.username,
        )
        .unwrap();
        assert!(pr.pr_number.starts_with("1011-"));
        assert_eq!(pr.total_amount, 14_000.0);
        assert_eq!(pr.item_count, 2);

        // PR is Unobligated and does NOT diminish Available
        let dash_before_obr = services::get_dashboard_summary(&conn).unwrap();
        assert_eq!(dash_before_obr.total_unobligated, 14_000.0);
        assert_eq!(dash_before_obr.total_obligated, 0.0);
        assert_eq!(dash_before_obr.total_available, 90_000.0);

        // 4. Over-budget check returns BudgetWarning unless forced
        let warn = services::convert_pr_to_obr(
            &mut conn,
            &ConvertPRToOBRRequest {
                pr_id: pr.pr_id,
                amount: 95_000.0,
                force_over_budget: false,
            },
            &encoder.username,
        )
        .unwrap();
        assert!(matches!(warn, ConvertPRToOBRResponse::BudgetWarning { .. }));

        // 5. Normal PR -> OBR conversion
        let created = services::convert_pr_to_obr(
            &mut conn,
            &ConvertPRToOBRRequest {
                pr_id: pr.pr_id,
                amount: 14_000.0,
                force_over_budget: false,
            },
            &encoder.username,
        )
        .unwrap();
        let obr = match created {
            ConvertPRToOBRResponse::Created { obr } => obr,
            _ => panic!("Expected OBR to be created"),
        };
        assert!(obr.obr_number.starts_with("OBR-"));
        assert_eq!(obr.amount, 14_000.0);

        let pr_after = services::get_pr(&conn, pr.pr_id).unwrap();
        assert_eq!(pr_after.status, "Served");

        let acc_after = services::get_account(&conn, acc.account_id).unwrap();
        assert_eq!(acc_after.available, 76_000.0);

        // 6. Create DV from OBR
        let dv = services::create_dv(
            &mut conn,
            &CreateDVRequest {
                obr_id: obr.obr_id,
                check_no: Some("CHK-0001".to_string()),
                check_date: Some("2026-11-17".to_string()),
            },
            &encoder.username,
        )
        .unwrap();
        assert!(dv.dv_number.starts_with("DV-"));
        let obr_after = services::get_obr(&conn, obr.obr_id).unwrap();
        assert_eq!(obr_after.status, "Paid");

        // 7. Recalculate & Dashboard
        let recalc = services::recalculate_all_available(&conn, &admin.username).unwrap();
        assert_eq!(recalc, 1);

        let dash = services::get_dashboard_summary(&conn).unwrap();
        assert_eq!(dash.total_budget, 100_000.0);
        assert_eq!(dash.total_unobligated, 0.0);
        assert_eq!(dash.total_available, 76_000.0);
        assert_eq!(dash.total_obligated, 14_000.0);
        assert_eq!(dash.pr_count, 1);
        assert_eq!(dash.obr_count, 1);
        assert_eq!(dash.dv_count, 1);

        // 8. Audit trail recorded
        let audits = services::list_audit_trail(&conn, &AuditFilter::default()).unwrap();
        assert!(audits.len() >= 5);

        // 9. Budget Summary report renders cleanly
        let summary_html = reports::render_budget_summary_report(&dash);
        assert!(summary_html.contains("BUDGET SUMMARY REPORT") || summary_html.contains("Budget Summary Report"));
        assert!(summary_html.contains("5-02-03-010"));
    }

    #[test]
    fn dev_seeding_is_idempotent() {
        let conn = db::open_in_memory().expect("in-memory db");
        db::seed_dev_data(&conn).unwrap();
        db::seed_dev_data(&conn).unwrap();
        let accounts = services::list_accounts(&conn).unwrap();
        assert_eq!(accounts.len(), 3);
    }

    #[test]
    fn delete_account_respects_references() {
        let mut conn = db::open_in_memory().expect("in-memory db");
        let unused = services::create_account(
            &conn,
            &CreateAccountRequest {
                account_code: "5-02-03-220".to_string(),
                account_name: None,
                budget: 50_000.0,
                contingency: false,
            },
            "admin",
        )
        .unwrap();
        services::delete_account(&conn, unused.account_id, "admin").unwrap();
        assert!(services::get_account(&conn, unused.account_id).is_err());

        let used = services::create_account(
            &conn,
            &CreateAccountRequest {
                account_code: "5-02-03-010".to_string(),
                account_name: None,
                budget: 50_000.0,
                contingency: false,
            },
            "admin",
        )
        .unwrap();
        services::create_pr(
            &mut conn,
            &SavePRRequest {
                pr_date: None,
                department: "PESO".to_string(),
                fund_source: "General Fund".to_string(),
                aip_code: None,
                account_code: "5-02-03-010".to_string(),
                payee: "Vendor".to_string(),
                purpose: "Test".to_string(),
                status: None,
                remarks: None,
                items: vec![PRItemInput {
                    unit: "pcs".to_string(),
                    description: "Item".to_string(),
                    quantity: 1.0,
                    unit_cost: 100.0,
                }],
            },
            "admin",
        )
        .unwrap();
        assert!(matches!(
            services::delete_account(&conn, used.account_id, "admin"),
            Err(services::ServiceError::Conflict(_))
        ));
    }

    #[test]
    fn direct_obr_and_augmentation_workflow() {
        let mut conn = db::open_in_memory().expect("in-memory db");
        let src = services::create_account(
            &conn,
            &CreateAccountRequest {
                account_code: "5-02-03-010".to_string(),
                account_name: None,
                budget: 100_000.0,
                contingency: false,
            },
            "admin",
        )
        .unwrap();
        let dst = services::create_account(
            &conn,
            &CreateAccountRequest {
                account_code: "5-02-03-210".to_string(),
                account_name: None,
                budget: 50_000.0,
                contingency: true,
            },
            "admin",
        )
        .unwrap();

        // Transfer 20,000 from src to dst (dst has 10% contingency -> +18,000 available)
        let aug = services::create_augmentation(
            &mut conn,
            &CreateAugmentationRequest {
                source_account_code: "5-02-03-010".to_string(),
                destination_account_code: "5-02-03-210".to_string(),
                amount: 20_000.0,
                justification: "Additional travel budget".to_string(),
                date: None,
            },
            "admin",
        )
        .unwrap();
        assert_eq!(aug.amount, 20_000.0);

        let src_after = services::get_account(&conn, src.account_id).unwrap();
        let dst_after = services::get_account(&conn, dst.account_id).unwrap();
        assert_eq!(src_after.budget, 80_000.0);
        assert_eq!(src_after.available, 80_000.0);
        assert_eq!(dst_after.budget, 70_000.0);
        assert_eq!(dst_after.available, 63_000.0);

        // Direct OBR without PR
        let res = services::create_direct_obr(
            &mut conn,
            &CreateDirectOBRRequest {
                account_code: "5-02-03-210".to_string(),
                payee: "Airline Co.".to_string(),
                purpose: "Official travel".to_string(),
                amount: 10_000.0,
                force_over_budget: false,
            },
            "encoder",
        )
        .unwrap();
        let obr = match res {
            ConvertPRToOBRResponse::Created { obr } => obr,
            _ => panic!("Expected direct OBR to be created"),
        };
        assert!(obr.pr_id.is_none());
        assert!(obr.pr_number.is_none());

        let dst_final = services::get_account(&conn, dst.account_id).unwrap();
        assert_eq!(dst_final.available, 53_000.0);
    }

    #[test]
    fn chart_of_accounts_seeded_from_coc_csv() {
        let conn = db::open_in_memory().expect("in-memory db");
        let coa = services::list_chart_of_accounts(&conn).unwrap();
        assert!(coa.len() >= 500, "Expected 500+ COA entries from coc.csv, got {}", coa.len());
        let cash_in_bank = coa.iter().find(|c| c.account_code == "1-01-02-010").unwrap();
        assert_eq!(
            cash_in_bank.account_name,
            "Cash in Bank - Local Currency, Current Account"
        );
        assert_eq!(cash_in_bank.category.as_deref(), Some("Assets"));
    }
}

