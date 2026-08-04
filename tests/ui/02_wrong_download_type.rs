//! Misuse 2: decoding an export as a type the template never declared.

use expensify::{Client, Credentials, Error, ExportTemplate, Json, ReportsQuery};

async fn run(client: &Client) -> Result<(), Error> {
    let template: ExportTemplate<Json<Vec<i64>>> = ExportTemplate::typed("...");
    let file = client
        .export_reports(&template, ReportsQuery::report_ids(["R1"]))
        .await?;

    let _rows: Vec<String> = client.download(&file).await?;
    Ok(())
}

fn main() {
    let _ = run(&Client::new(Credentials::new("id", "secret")));
}
