use time::Date;

use crate::client::Client;
use crate::error::Error;
use crate::types::{Currency, Money, PolicyId, ReportId, TaxRateId, TransactionId};
use crate::BoxFuture;

/// Tax applied to an expense. `rate_id` values come from the Policy
/// Getter (`fields: ["tax"]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpenseTax {
    rate_id: TaxRateId,
    amount_cents: Option<i64>,
}

impl ExpenseTax {
    pub fn new(rate_id: impl Into<TaxRateId>) -> Self {
        Self { rate_id: rate_id.into(), amount_cents: None }
    }

    /// Explicit tax amount, for partially taxed expenses.
    pub fn amount_cents(mut self, cents: i64) -> Self {
        self.amount_cents = Some(cents);
        self
    }
}

/// One expense for the Expense Creator (`transactionList` entry).
/// Required fields in the constructor, the rest fluent.
#[derive(Clone, Debug)]
pub struct Expense {
    merchant: String,
    date: Date,
    amount: Money,
    external_id: Option<String>,
    category: Option<String>,
    tag: Option<String>,
    billable: Option<bool>,
    reimbursable: Option<bool>,
    comment: Option<String>,
    report_id: Option<ReportId>,
    policy_id: Option<PolicyId>,
    tax: Option<ExpenseTax>,
}

impl Expense {
    pub fn new(merchant: impl Into<String>, date: Date, amount: Money) -> Self {
        Self {
            merchant: merchant.into(),
            date,
            amount,
            external_id: None,
            category: None,
            tag: None,
            billable: None,
            reimbursable: None,
            comment: None,
            report_id: None,
            policy_id: None,
            tax: None,
        }
    }

    /// Caller-chosen unique ID, surfaced again on export.
    pub fn external_id(mut self, id: impl Into<String>) -> Self {
        self.external_id = Some(id.into());
        self
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn billable(mut self, billable: bool) -> Self {
        self.billable = Some(billable);
        self
    }

    pub fn reimbursable(mut self, reimbursable: bool) -> Self {
        self.reimbursable = Some(reimbursable);
        self
    }

    pub fn comment(mut self, comment: impl Into<String>) -> Self {
        self.comment = Some(comment.into());
        self
    }

    /// Attach to an existing report.
    pub fn report_id(mut self, id: impl Into<ReportId>) -> Self {
        self.report_id = Some(id.into());
        self
    }

    /// Policy the tax rate belongs to; pair with [`Expense::tax`].
    pub fn policy_id(mut self, id: impl Into<PolicyId>) -> Self {
        self.policy_id = Some(id.into());
        self
    }

    pub fn tax(mut self, tax: ExpenseTax) -> Self {
        self.tax = Some(tax);
        self
    }
}

/// Expense Creator (`type: "create"`, `inputSettings.type: "expenses"`).
#[must_use = "actions do nothing until awaited"]
pub struct CreateExpensesAction {
    client: Client,
    expenses: Vec<Expense>,
    employee_email: Option<String>,
}

impl CreateExpensesAction {
    pub(crate) fn new(client: Client, expenses: Vec<Expense>) -> Self {
        Self { client, expenses, employee_email: None }
    }

    /// Create in another user's account. Restricted: requires advanced
    /// permissions granted by Expensify. Default: the credential owner's
    /// account.
    pub fn employee_email(mut self, email: impl Into<String>) -> Self {
        self.employee_email = Some(email.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct CreatedTransaction {
    pub transaction_id: TransactionId,
    pub merchant: String,
    pub created: Date,
    pub amount_cents: i64,
    pub currency: Currency,
}

impl IntoFuture for CreateExpensesAction {
    type Output = Result<Vec<CreatedTransaction>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let _ = self;
            todo!()
        })
    }
}
