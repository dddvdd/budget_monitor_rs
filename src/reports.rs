use crate::models::{
    format_currency, pesos_to_cents, AuditEntry, DashboardSummary, DisbursementVoucher,
    ObligationRequest, PurchaseRequest,
};

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn report_shell(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<style>
  @page {{ size: A4 portrait; margin: 0.5in; }}
  body {{ font-family: "Segoe UI", Arial, sans-serif; color: #111; margin: 0; padding: 24px; font-size: 13px; }}
  .sheet {{ max-width: 780px; margin: 0 auto; border: 1.5px solid #222; padding: 24px; }}
  .header {{ text-align: center; border-bottom: 2px solid #222; padding-bottom: 10px; margin-bottom: 16px; }}
  .header h1 {{ margin: 0; font-size: 18px; letter-spacing: 0.5px; text-transform: uppercase; }}
  .header p {{ margin: 4px 0 0; font-size: 12px; color: #444; }}
  .meta-grid {{ display: grid; grid-template-columns: 1fr 1fr; gap: 8px 16px; margin-bottom: 14px; }}
  .meta-item {{ border: 1px solid #ccc; padding: 6px 10px; }}
  .meta-label {{ font-size: 11px; color: #555; text-transform: uppercase; }}
  .meta-val {{ font-weight: 600; margin-top: 2px; }}
  .box {{ border: 1px solid #ccc; padding: 10px; margin-bottom: 14px; }}
  table {{ width: 100%; border-collapse: collapse; margin-bottom: 14px; }}
  th, td {{ border: 1px solid #333; padding: 6px 8px; text-align: left; }}
  th {{ background: #f3f4f6; font-size: 12px; }}
  .num {{ text-align: right; font-variant-numeric: tabular-nums; }}
  .sig-grid {{ display: grid; grid-template-columns: 1fr 1fr; gap: 24px; margin-top: 24px; }}
  .sig-box {{ text-align: center; padding-top: 28px; }}
  .sig-line {{ border-top: 1px solid #222; padding-top: 4px; font-weight: 600; }}
  .toolbar {{ max-width: 780px; margin: 0 auto 12px; text-align: right; }}
  .btn {{ background: #1d4ed8; color: #fff; border: none; padding: 8px 14px; border-radius: 4px; cursor: pointer; font-size: 13px; }}
  @media print {{ .toolbar {{ display: none; }} body {{ padding: 0; }} .sheet {{ border: none; }} }}
</style>
</head>
<body>
<div class="toolbar"><button class="btn" onclick="window.print()">Print / Save as PDF</button></div>
<div class="sheet">
{body}
</div>
</body>
</html>"#,
        title = escape_html(title),
        body = body
    )
}

pub fn render_pr_report(pr: &PurchaseRequest) -> String {
    let items_rows = pr
        .items
        .as_ref()
        .map(|items| {
            items
                .iter()
                .map(|it| {
                    format!(
                        "<tr><td class=\"num\">{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                        it.line_number,
                        escape_html(&it.unit),
                        escape_html(&it.description),
                        it.quantity,
                        format_currency(pesos_to_cents(it.unit_cost)),
                        format_currency(pesos_to_cents(it.total_cost)),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let body = format!(
        r#"<div class="header">
  <h1>Purchase Request</h1>
  <p>Provincial Government of Cagayan — Public Employment Service Office (PESO)</p>
</div>
<div class="meta-grid">
  <div class="meta-item"><div class="meta-label">PR Number</div><div class="meta-val">{pr_number}</div></div>
  <div class="meta-item"><div class="meta-label">Date</div><div class="meta-val">{pr_date}</div></div>
  <div class="meta-item"><div class="meta-label">Department / Office</div><div class="meta-val">{department}</div></div>
  <div class="meta-item"><div class="meta-label">Fund Source</div><div class="meta-val">{fund_source}</div></div>
  <div class="meta-item"><div class="meta-label">Account Code</div><div class="meta-val">{account_code}</div></div>
  <div class="meta-item"><div class="meta-label">AIP Code</div><div class="meta-val">{aip_code}</div></div>
  <div class="meta-item"><div class="meta-label">Payee</div><div class="meta-val">{payee}</div></div>
  <div class="meta-item"><div class="meta-label">Status</div><div class="meta-val">{status}</div></div>
</div>
<table>
  <thead>
    <tr><th class="num">#</th><th>Unit</th><th>Item Description</th><th class="num">Qty</th><th class="num">Unit Cost</th><th class="num">Total Cost</th></tr>
  </thead>
  <tbody>
    {items_rows}
    <tr>
      <td colspan="5" class="num"><strong>TOTAL AMOUNT</strong></td>
      <td class="num"><strong>{total_amount}</strong></td>
    </tr>
  </tbody>
</table>
<div class="box"><div class="meta-label">Purpose</div><div>{purpose}</div></div>
<div class="sig-grid">
  <div class="sig-box"><div class="sig-line">{created_by}</div><div>Requested By / Prepared By</div></div>
  <div class="sig-box"><div class="sig-line">Approving Officer</div><div>Approved By</div></div>
</div>"#,
        pr_number = escape_html(&pr.pr_number),
        pr_date = escape_html(&pr.pr_date),
        department = escape_html(&pr.department),
        fund_source = escape_html(&pr.fund_source),
        account_code = escape_html(&pr.account_code),
        aip_code = escape_html(pr.aip_code.as_deref().unwrap_or("—")),
        payee = escape_html(&pr.payee),
        status = escape_html(&pr.status),
        items_rows = items_rows,
        total_amount = format_currency(pesos_to_cents(pr.total_amount)),
        purpose = escape_html(&pr.purpose),
        created_by = escape_html(&pr.created_by),
    );

    report_shell(&format!("PR {}", pr.pr_number), &body)
}

pub fn render_obr_report(obr: &ObligationRequest) -> String {
    let body = format!(
        r#"<div class="header">
  <h1>Obligation Request</h1>
  <p>Provincial Government of Cagayan — Public Employment Service Office (PESO)</p>
</div>
<div class="meta-grid">
  <div class="meta-item"><div class="meta-label">OBR No.</div><div class="meta-val">{obr_number}</div></div>
  <div class="meta-item"><div class="meta-label">Date</div><div class="meta-val">{obr_date}</div></div>
  <div class="meta-item"><div class="meta-label">PR Reference</div><div class="meta-val">{pr_number}</div></div>
  <div class="meta-item"><div class="meta-label">Account Code</div><div class="meta-val">{account_code}</div></div>
  <div class="meta-item"><div class="meta-label">Payee</div><div class="meta-val">{payee}</div></div>
  <div class="meta-item"><div class="meta-label">Status</div><div class="meta-val">{status}</div></div>
</div>
<div class="box"><div class="meta-label">Purpose / Particulars</div><div style="margin-top:4px">{purpose}</div></div>
<div class="box"><div class="meta-label">Obligated Amount</div><div class="meta-val" style="font-size:16px">{amount}</div></div>
<div class="box">
  <div class="meta-label">Certification</div>
  <p style="margin:6px 0 0">I certify that the above obligation is correct, lawful, and supported by valid allotment.</p>
  <div class="sig-grid">
    <div class="sig-box"><div class="sig-line">{created_by}</div><div>Requested By</div></div>
    <div class="sig-box"><div class="sig-line">Budget Officer / Approving Officer</div><div>Certified By</div></div>
  </div>
</div>"#,
        obr_number = escape_html(&obr.obr_number),
        obr_date = escape_html(&obr.obr_date),
        pr_number = escape_html(obr.pr_number.as_deref().unwrap_or("Direct (No PR)")),
        account_code = escape_html(&obr.account_code),
        payee = escape_html(&obr.payee),
        status = escape_html(&obr.status),
        purpose = escape_html(&obr.purpose),
        amount = format_currency(pesos_to_cents(obr.amount)),
        created_by = escape_html(&obr.created_by),
    );

    report_shell(&format!("OBR {}", obr.obr_number), &body)
}

pub fn render_dv_report(dv: &DisbursementVoucher, account_code: &str) -> String {
    let amount_str = format_currency(pesos_to_cents(dv.amount));
    let body = format!(
        r#"<div class="header">
  <h1>Disbursement Voucher</h1>
  <p>Provincial Government of Cagayan — Public Employment Service Office (PESO)</p>
</div>
<div class="meta-grid">
  <div class="meta-item"><div class="meta-label">DV No.</div><div class="meta-val">{dv_number}</div></div>
  <div class="meta-item"><div class="meta-label">Date</div><div class="meta-val">{dv_date}</div></div>
  <div class="meta-item"><div class="meta-label">OBR Reference</div><div class="meta-val">{obr_number}</div></div>
  <div class="meta-item"><div class="meta-label">Payee</div><div class="meta-val">{payee}</div></div>
  <div class="meta-item"><div class="meta-label">Check No.</div><div class="meta-val">{check_no}</div></div>
  <div class="meta-item"><div class="meta-label">Check Date</div><div class="meta-val">{check_date}</div></div>
</div>
<table>
  <thead>
    <tr><th>Account Code</th><th class="num">Debit</th><th class="num">Credit</th></tr>
  </thead>
  <tbody>
    <tr><td>{account_code}</td><td class="num">{amount}</td><td class="num"></td></tr>
    <tr><td>Cash in Bank</td><td class="num"></td><td class="num">{amount}</td></tr>
    <tr><td><strong>Total</strong></td><td class="num"><strong>{amount}</strong></td><td class="num"><strong>{amount}</strong></td></tr>
  </tbody>
</table>
<div class="box">
  <div class="meta-label">Certification</div>
  <p style="margin:6px 0 0">I certify that the above disbursement is correct, lawful, and properly supported.</p>
  <div class="sig-grid">
    <div class="sig-box"><div class="sig-line">{created_by}</div><div>Prepared By</div></div>
    <div class="sig-box"><div class="sig-line">Provincial Treasurer / Approving Officer</div><div>Approved for Payment</div></div>
  </div>
</div>"#,
        dv_number = escape_html(&dv.dv_number),
        dv_date = escape_html(&dv.dv_date),
        obr_number = escape_html(&dv.obr_number),
        payee = escape_html(&dv.payee),
        check_no = escape_html(dv.check_no.as_deref().unwrap_or("—")),
        check_date = escape_html(dv.check_date.as_deref().unwrap_or("—")),
        account_code = escape_html(account_code),
        amount = amount_str,
        created_by = escape_html(&dv.created_by),
    );

    report_shell(&format!("DV {}", dv.dv_number), &body)
}

pub fn render_budget_summary_report(summary: &DashboardSummary) -> String {
    let rows = summary
        .accounts
        .iter()
        .map(|a| {
            let row_style = if a.contingency {
                " style=\"background:#e0f2fe\""
            } else {
                ""
            };
            format!(
                "<tr{}><td>{} <small style=\"color:#555\">({})</small></td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\"><strong>{}</strong></td><td>{}</td></tr>",
                row_style,
                escape_html(&a.account_name),
                escape_html(&a.account_code),
                format_currency(pesos_to_cents(a.budget)),
                format_currency(pesos_to_cents(a.unobligated)),
                format_currency(pesos_to_cents(a.obligated)),
                format_currency(pesos_to_cents(a.available)),
                escape_html(&a.status),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let body = format!(
        r#"<div class="header">
  <h1>Budget Summary Report</h1>
  <p>Provincial Government of Cagayan — Public Employment Service Office (PESO)</p>
  <p style="font-size:11px;color:#555">Generated: {updated_at} · Light blue rows have 10% Contingency</p>
</div>
<div class="meta-grid">
  <div class="meta-item"><div class="meta-label">Total Budget</div><div class="meta-val">{total_budget}</div></div>
  <div class="meta-item"><div class="meta-label">Total Available</div><div class="meta-val">{total_available}</div></div>
  <div class="meta-item"><div class="meta-label">Budgetary Reservation – Unobligated (PR)</div><div class="meta-val">{total_unobligated}</div></div>
  <div class="meta-item"><div class="meta-label">Total Obligated (OBR/DV)</div><div class="meta-val">{total_obligated}</div></div>
</div>
<table>
  <thead>
    <tr>
      <th>Account Name</th><th class="num">Budget</th><th class="num">Unobligated</th>
      <th class="num">Obligated</th><th class="num">Available</th><th>Status</th>
    </tr>
  </thead>
  <tbody>
    {rows}
    <tr>
      <td class="num"><strong>GRAND TOTAL</strong></td>
      <td class="num"><strong>{total_budget}</strong></td>
      <td class="num"><strong>{total_unobligated}</strong></td>
      <td class="num"><strong>{total_obligated}</strong></td>
      <td class="num"><strong>{total_available}</strong></td>
      <td></td>
    </tr>
  </tbody>
</table>"#,
        updated_at = escape_html(&summary.updated_at),
        total_budget = format_currency(pesos_to_cents(summary.total_budget)),
        total_available = format_currency(pesos_to_cents(summary.total_available)),
        total_unobligated = format_currency(pesos_to_cents(summary.total_unobligated)),
        total_obligated = format_currency(pesos_to_cents(summary.total_obligated)),
        rows = rows,
    );

    report_shell("Budget Summary Report", &body)
}

fn csv_cell(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn render_audit_csv(entries: &[AuditEntry]) -> String {
    let mut out = String::from("AuditID,Timestamp,Username,Action,TableName,RecordID,Details\n");
    for e in entries {
        out.push_str(&format!(
            "{},{},{},{},{},{},{}\n",
            e.audit_id,
            csv_cell(&e.timestamp),
            csv_cell(&e.username),
            csv_cell(&e.action),
            csv_cell(&e.table_name),
            e.record_id,
            csv_cell(e.details.as_deref().unwrap_or("")),
        ));
    }
    out
}

