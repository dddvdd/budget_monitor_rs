# Budget Monitoring System (Rust)

A standalone, high-performance Rust recreation of the Microsoft Access / VBA Budget Monitoring and Procurement System for the **Provincial Government of Cagayan — Public Employment Service Office (PESO)**.

---

## 🚀 How to Use the Project

### Option 1: One-Click Launcher (Easiest — Windows)
1. Double-click **`start.bat`**.
2. It automatically starts the server and opens your browser to **`http://localhost:8080`**.
3. To stop the application, close the terminal window.

### Option 2: Run the Standalone `.exe` Directly (No Rust or Access Required!)
Because the web server, SQLite engine, frontend interface, and Chart of Accounts are all compiled into a single binary:
1. Double-click **`budget_monitor.exe`** (or run `.\budget_monitor.exe` in PowerShell/CMD).
2. Open your web browser to **`http://localhost:8080`**.
3. *Note: You can copy just `budget_monitor.exe` (~3.8 MB) to a USB drive or any other Windows PC and double-click it. It will auto-create `budget_monitor.db` and run with zero external dependencies.*

### Option 3: Run with Rust / Cargo (From Source)
If you have the Rust toolchain installed:

```powershell
# Run with sample development budget accounts pre-loaded
cargo run -- --dev --bind 127.0.0.1:8080

# Or run with a clean fresh database
cargo run -- --bind 127.0.0.1:8080
```

Then open your browser to **`http://localhost:8080`**.

---

## 🔑 Default Login Credentials

| Role | Username | Password | Capabilities |
|---|---|---|---|
| **Admin** | `admin` | `admin123` | Full access (Budget Accounts, Augmentations, PR/OBR/DV, Audit Trail, Recalculate) |
| **Encoder** | `encoder` | `encoder123` | Operational access (Create/Edit PR, Convert to OBR, Direct OBR, Create DV) |

---

## 🌐 Production & LAN Multi-User Hosting

To host the system on your office local network so up to 5+ staff PCs can connect concurrently:

```powershell
.\budget_monitor.exe --bind 0.0.0.0:8080
```
*(Or with Cargo: `cargo run --release -- --bind 0.0.0.0:8080`)*

Other PCs on the same office Wi-Fi / LAN can then open:
👉 **`http://<SERVER-IP-ADDRESS>:8080`**

---

## 🧪 Testing

Run the automated in-memory integration test suite (tests login, budget ceiling checks, PR → OBR → DV lifecycle, augmentations, and audit logging):

```powershell
cargo test
```

To build a fresh release `.exe` at any time:

```powershell
cargo build --release
```

---

## 🏗️ Architecture & Features

- **Self-Contained Backend**: `axum` HTTP server + `rusqlite` with SQLite compiled directly into the binary (WAL mode enabled for safe concurrent writes).
- **Embedded Web UI**: Single-page application compiled into the binary via `include_str!` (responsive HTML5, CSS3, ES6).
- **Official Chart of Accounts**: Bundled with 511 official government accounts (`data/coc.csv`) with instant dual-code/name search autocomplete.
- **Budgetary Tracking**:
  - **Unobligated**: Budgetary Reservation from active Purchase Requests (PRs).
  - **Obligated**: Dedicated funds from Obligation Requests (OBRs) and Disbursement Vouchers (DVs).
  - **Contingency Protection**: Accounts with 10% contingency reservation highlighted in light blue. Only Obligated amounts diminish the Available balance.
- **Budget Augmentations**: Transfer funds between accounts with automated source balance checks and destination contingency recalculations.
- **Direct & Converted OBRs**: Convert PR to OBR or create Direct OBRs with over-budget warning guards.
- **Printable Vouchers & Reports**: Print-ready HTML templates for PR, OBR, DV, and complete Budget Summary Reports.
- **Audit Trail**: Every action logged with timestamp, user, action, and details; exportable to CSV.

---

## 📂 Mapping from Original Access / VBA Project

| Original (Access / VBA / SQL) | Rust Recreation |
|---|---|
| `SQL/01_tables.sql`, `04_sequences.sql` | `src/db.rs` |
| `SQL/02_queries.sql`, `03_maintenance.sql` | `src/services.rs` |
| `VBA/modGlobals.bas`, `modDashboard.bas`, `modPR.bas`, `modOBR.bas`, `modDV.bas` | `src/services.rs`, `src/handlers.rs` |
| `VBA/Forms/frmEditAccount.cls`, `frmAuditTrail.cls`, Access Forms | `static/index.html` |
| `Notes/rptOBR_design.md`, `rptDV_design.md`, `rptPR` | `src/reports.rs` |
| `coc.csv` | `data/coc.csv`, `src/db.rs` |
