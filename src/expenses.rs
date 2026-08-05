use time::Date;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::Error;
use crate::types::{Currency, Money, PolicyId, ReportId, TaxRateId, TransactionId};
use crate::wire;

/// Tax applied to an expense. `rate_id` values come from the Policy
/// Getter (`fields: ["tax"]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpenseTax {
    pub(crate) rate_id: TaxRateId,
    pub(crate) amount_cents: Option<i64>,
}

impl ExpenseTax {
    /// Apply the policy's rate identified by `rate_id`; see
    /// [`TaxRate::rate_id`](crate::TaxRate::rate_id).
    pub fn new(rate_id: impl Into<TaxRateId>) -> Self {
        Self {
            rate_id: rate_id.into(),
            amount_cents: None,
        }
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
    pub(crate) merchant: String,
    pub(crate) date: Date,
    pub(crate) amount: Money,
    pub(crate) external_id: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) billable: Option<bool>,
    pub(crate) reimbursable: Option<bool>,
    pub(crate) comment: Option<String>,
    pub(crate) report_id: Option<ReportId>,
    pub(crate) policy_id: Option<PolicyId>,
    pub(crate) tax: Option<ExpenseTax>,
}

impl Expense {
    /// The three fields Expensify always requires.
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

    /// Policy category name.
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Policy tag name.
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    /// Mark billable to a client.
    pub fn billable(mut self, billable: bool) -> Self {
        self.billable = Some(billable);
        self
    }

    /// Mark reimbursable to the employee.
    pub fn reimbursable(mut self, reimbursable: bool) -> Self {
        self.reimbursable = Some(reimbursable);
        self
    }

    /// Free-text comment.
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

    /// Attach tax; requires [`Expense::policy_id`] or an existing report to
    /// resolve the rate.
    pub fn tax(mut self, tax: ExpenseTax) -> Self {
        self.tax = Some(tax);
        self
    }
}

/// Expense Creator (`type: "create"`, `inputSettings.type: "expenses"`).
///
/// The employee the expenses belong to is a required argument of
/// [`Client::create_expenses`](crate::Client::create_expenses), not a setter:
/// Expensify rejects the job without it (410, `'employeeEmail' parameter is
/// missing or malformed`), with or without a policy on the expenses.
#[must_use = "actions do nothing until awaited"]
pub struct CreateExpensesAction {
    pub(crate) client: Client,
    pub(crate) expenses: Vec<Expense>,
    pub(crate) employee_email: String,
}

impl CreateExpensesAction {
    pub(crate) fn new(client: Client, employee_email: String, expenses: Vec<Expense>) -> Self {
        Self {
            client,
            expenses,
            employee_email,
        }
    }
}

/// One created expense, as echoed back by Expensify.
///
/// The response also carries `comment`, `tag`, `category` and `mcc`, which
/// echo the request (or Expensify's defaults for it, e.g. `"Uncategorized"`)
/// rather than telling the caller anything new. They are not modelled; the
/// raw body is one `ClientBuilder::observe` away if you need them.
#[derive(Clone, Debug)]
pub struct CreatedTransaction {
    /// Assigned identifier.
    pub transaction_id: TransactionId,
    /// The report the expense landed in.
    ///
    /// **Undocumented, and the reason this field exists:** an expense created
    /// without [`Expense::report_id`] is not left loose. Expensify opens a
    /// report for it and names that report here, so this is the only way to
    /// learn where the expense went short of a separate export.
    ///
    /// **`Option` deliberately — do not tidy this away.** `reportID` has been
    /// present on every observed response, so the type looks needlessly weak.
    /// It is not: this field describes a side effect rather than the
    /// transaction, and making it required would turn a response that omitted
    /// it into a decode error on an expense that *was created*. An error that
    /// does not mean "nothing happened" is the worst failure this API can
    /// hand a caller — retrying duplicates the expense, and not retrying
    /// leaves one they cannot find. `None` costs them only this knowledge.
    pub report_id: Option<ReportId>,
    /// Merchant as stored.
    pub merchant: String,
    /// Expense date (`created` on the wire).
    pub created: Date,
    /// Amount in integer cents.
    pub amount_cents: i64,
    /// Currency of `amount_cents`.
    pub currency: Currency,
}

impl IntoFuture for CreateExpensesAction {
    type Output = Result<Vec<CreatedTransaction>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::create_expenses(&self);
            let response = self.client.send(request).await?;
            wire::created_transactions(response)
        })
    }
}
