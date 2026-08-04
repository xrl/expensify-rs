//! Policy Getter (`type: "get"`, `inputSettings.type: "policy"`).
//!
//! The `fields` list a caller requests decides which parts of the response
//! are populated. That runtime fact is lifted into the type system: each
//! `with_*` call flips one type-level flag from [`Omitted`] to
//! [`Fetched`], and the returned [`Policy`] has a real field where the
//! flag is `Fetched` and an inert [`NotFetched`] placeholder where it is
//! not. Reading data you did not request is a compile error, not an
//! `unwrap`.

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::DeserializeOwned;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::{DecodeError, Error};
use crate::policy::model::{Category, PolicyEmployee, PolicyTag, ReportField, TaxConfig};
use crate::types::PolicyId;
use crate::wire;

mod sealed {
    pub trait Sealed {}
}

/// Everything a fetch-gated payload must satisfy.
///
/// Blanket-implemented; the bound exists so [`Policy`] can derive `Debug`
/// and `Clone` without a where-clause per field.
pub trait Payload: fmt::Debug + Clone + Send + Sync + 'static {}

impl<T: fmt::Debug + Clone + Send + Sync + 'static> Payload for T {}

/// Type-level flag: was this policy field requested? Sealed; the only
/// states are [`Fetched`] and [`Omitted`].
pub trait FetchState: sealed::Sealed + Send + Sync + 'static {
    /// `T` when fetched, [`NotFetched`] when omitted.
    type Wrap<T: Payload>: Payload;

    /// Deserialization hook, so one generic `IntoFuture` impl serves all 32
    /// combinations of states.
    #[doc(hidden)]
    fn extract<T>(
        field: &'static str,
        value: Option<serde_json::Value>,
    ) -> Result<Self::Wrap<T>, Error>
    where
        T: DeserializeOwned + Payload;
}

/// The field was requested; the payload is present, no `Option`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fetched;

/// The field was not requested. Its slot in [`Policy`] is [`NotFetched`],
/// which has no data and no methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Omitted;

/// Placeholder occupying unrequested [`Policy`] fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NotFetched;

impl sealed::Sealed for Fetched {}
impl sealed::Sealed for Omitted {}

impl FetchState for Fetched {
    type Wrap<T: Payload> = T;

    fn extract<T>(field: &'static str, value: Option<serde_json::Value>) -> Result<T, Error>
    where
        T: DeserializeOwned + Payload,
    {
        let value = value.ok_or_else(|| {
            Error::from(DecodeError::custom(format!(
                "policy response is missing requested field `{field}`"
            )))
        })?;
        serde_json::from_value(value).map_err(|err| DecodeError::Json(err).into())
    }
}

impl FetchState for Omitted {
    type Wrap<T: Payload> = NotFetched;

    fn extract<T>(
        _field: &'static str,
        _value: Option<serde_json::Value>,
    ) -> Result<NotFetched, Error>
    where
        T: DeserializeOwned + Payload,
    {
        Ok(NotFetched)
    }
}

/// One policy from the getter response. Each type parameter records
/// whether the corresponding field was requested.
#[derive(Clone, Debug)]
pub struct Policy<
    Cats: FetchState = Omitted,
    Fields: FetchState = Omitted,
    Tags: FetchState = Omitted,
    Tax: FetchState = Omitted,
    Emps: FetchState = Omitted,
> {
    /// Expense categories, present when `with_categories()` was called.
    pub categories: Cats::Wrap<Vec<Category>>,
    /// Report fields, present when `with_report_fields()` was called.
    pub report_fields: Fields::Wrap<Vec<ReportField>>,
    /// Tags, present when `with_tags()` was called.
    pub tags: Tags::Wrap<Vec<PolicyTag>>,
    /// `None` when the policy has no tax configuration (the API returns
    /// `"tax": {}`); this `Option` is data-dependent, not request-dependent.
    pub tax: Tax::Wrap<Option<TaxConfig>>,
    /// Policy members, present when `with_employees()` was called.
    pub employees: Emps::Wrap<Vec<PolicyEmployee>>,
}

/// Return type of an awaited [`GetPoliciesAction`], keyed by policy ID.
pub type Policies<Cats, Fields, Tags, Tax, Emps> =
    HashMap<PolicyId, Policy<Cats, Fields, Tags, Tax, Emps>>;

/// First stage of the Policy Getter: not awaitable. At least one `with_*`
/// selection is required (the API rejects an empty `fields` list), so
/// `IntoFuture` only exists on [`GetPoliciesAction`], which each `with_*`
/// returns.
#[must_use = "select at least one field with `with_*`, then await"]
pub struct GetPoliciesBuilder {
    client: Client,
    ids: Vec<PolicyId>,
    user_email: Option<String>,
}

impl GetPoliciesBuilder {
    pub(crate) fn new(client: Client, ids: Vec<PolicyId>) -> Self {
        Self {
            client,
            ids,
            user_email: None,
        }
    }

    /// Act on behalf of another user (`userEmail`). Requires a prior
    /// third-party access grant from that user or their domain.
    pub fn on_behalf_of(mut self, email: impl Into<String>) -> Self {
        self.user_email = Some(email.into());
        self
    }

    fn action(self) -> GetPoliciesAction {
        GetPoliciesAction {
            client: self.client,
            ids: self.ids,
            fields: Vec::new(),
            user_email: self.user_email,
            _marker: PhantomData,
        }
    }

    /// Request expense categories.
    pub fn with_categories(self) -> GetPoliciesAction<Fetched, Omitted, Omitted, Omitted, Omitted> {
        self.action().with_categories()
    }

    /// Request report fields.
    pub fn with_report_fields(
        self,
    ) -> GetPoliciesAction<Omitted, Fetched, Omitted, Omitted, Omitted> {
        self.action().with_report_fields()
    }

    /// Request tags.
    pub fn with_tags(self) -> GetPoliciesAction<Omitted, Omitted, Fetched, Omitted, Omitted> {
        self.action().with_tags()
    }

    /// Request the tax configuration.
    pub fn with_tax(self) -> GetPoliciesAction<Omitted, Omitted, Omitted, Fetched, Omitted> {
        self.action().with_tax()
    }

    /// Request policy members.
    pub fn with_employees(self) -> GetPoliciesAction<Omitted, Omitted, Omitted, Omitted, Fetched> {
        self.action().with_employees()
    }
}

/// Awaitable Policy Getter with at least one field selected. Further
/// `with_*` calls are available only for fields still [`Omitted`], so each
/// selection is made at most once.
#[must_use = "actions do nothing until awaited"]
pub struct GetPoliciesAction<
    Cats: FetchState = Omitted,
    Fields: FetchState = Omitted,
    Tags: FetchState = Omitted,
    Tax: FetchState = Omitted,
    Emps: FetchState = Omitted,
> {
    client: Client,
    ids: Vec<PolicyId>,
    /// Wire `fields` values accumulated by `with_*` calls; the type
    /// parameters shape only the response.
    fields: Vec<&'static str>,
    user_email: Option<String>,
    // `fn() -> _` keeps the action `Send + Sync` regardless of the states.
    #[allow(clippy::type_complexity)]
    _marker: PhantomData<fn() -> (Cats, Fields, Tags, Tax, Emps)>,
}

impl<Cats, Fields, Tags, Tax, Emps> GetPoliciesAction<Cats, Fields, Tags, Tax, Emps>
where
    Cats: FetchState,
    Fields: FetchState,
    Tags: FetchState,
    Tax: FetchState,
    Emps: FetchState,
{
    /// See [`GetPoliciesBuilder::on_behalf_of`].
    pub fn on_behalf_of(mut self, email: impl Into<String>) -> Self {
        self.user_email = Some(email.into());
        self
    }

    fn cast<C2, F2, T2, X2, E2>(
        mut self,
        field: &'static str,
    ) -> GetPoliciesAction<C2, F2, T2, X2, E2>
    where
        C2: FetchState,
        F2: FetchState,
        T2: FetchState,
        X2: FetchState,
        E2: FetchState,
    {
        self.fields.push(field);
        GetPoliciesAction {
            client: self.client,
            ids: self.ids,
            fields: self.fields,
            user_email: self.user_email,
            _marker: PhantomData,
        }
    }
}

impl<Fields, Tags, Tax, Emps> GetPoliciesAction<Omitted, Fields, Tags, Tax, Emps>
where
    Fields: FetchState,
    Tags: FetchState,
    Tax: FetchState,
    Emps: FetchState,
{
    /// Request expense categories.
    pub fn with_categories(self) -> GetPoliciesAction<Fetched, Fields, Tags, Tax, Emps> {
        self.cast("categories")
    }
}

impl<Cats, Tags, Tax, Emps> GetPoliciesAction<Cats, Omitted, Tags, Tax, Emps>
where
    Cats: FetchState,
    Tags: FetchState,
    Tax: FetchState,
    Emps: FetchState,
{
    /// Request report fields.
    pub fn with_report_fields(self) -> GetPoliciesAction<Cats, Fetched, Tags, Tax, Emps> {
        self.cast("reportFields")
    }
}

impl<Cats, Fields, Tax, Emps> GetPoliciesAction<Cats, Fields, Omitted, Tax, Emps>
where
    Cats: FetchState,
    Fields: FetchState,
    Tax: FetchState,
    Emps: FetchState,
{
    /// Request tags.
    pub fn with_tags(self) -> GetPoliciesAction<Cats, Fields, Fetched, Tax, Emps> {
        self.cast("tags")
    }
}

impl<Cats, Fields, Tags, Emps> GetPoliciesAction<Cats, Fields, Tags, Omitted, Emps>
where
    Cats: FetchState,
    Fields: FetchState,
    Tags: FetchState,
    Emps: FetchState,
{
    /// Request the tax configuration.
    pub fn with_tax(self) -> GetPoliciesAction<Cats, Fields, Tags, Fetched, Emps> {
        self.cast("tax")
    }
}

impl<Cats, Fields, Tags, Tax> GetPoliciesAction<Cats, Fields, Tags, Tax, Omitted>
where
    Cats: FetchState,
    Fields: FetchState,
    Tags: FetchState,
    Tax: FetchState,
{
    /// Request policy members.
    pub fn with_employees(self) -> GetPoliciesAction<Cats, Fields, Tags, Tax, Fetched> {
        self.cast("employees")
    }
}

impl<Cats, Fields, Tags, Tax, Emps> IntoFuture for GetPoliciesAction<Cats, Fields, Tags, Tax, Emps>
where
    Cats: FetchState,
    Fields: FetchState,
    Tags: FetchState,
    Tax: FetchState,
    Emps: FetchState,
{
    type Output = Result<Policies<Cats, Fields, Tags, Tax, Emps>, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let request = wire::get_policies(&self.ids, &self.fields, self.user_email.as_deref());
            let response = self.client.send(request).await?;

            let mut policies = Policies::new();
            for (id, mut info) in wire::policy_info(response)? {
                let mut section = |key: &str| info.as_object_mut().and_then(|map| map.remove(key));
                let categories = section("categories");
                let report_fields = section("reportFields");
                let tags = section("tags");
                let tax = section("tax").map(wire::normalize_tax);
                let employees = section("employees");

                policies.insert(
                    id,
                    Policy {
                        categories: Cats::extract("categories", categories)?,
                        report_fields: Fields::extract("reportFields", report_fields)?,
                        tags: Tags::extract("tags", tags)?,
                        tax: Tax::extract("tax", tax)?,
                        employees: Emps::extract("employees", employees)?,
                    },
                );
            }
            Ok(policies)
        })
    }
}
