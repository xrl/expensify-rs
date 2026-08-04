//! Policy Getter (`type: "get"`, `inputSettings.type: "policy"`).
//!
//! The `fields` list a caller requests decides which parts of the response
//! are populated. That runtime fact is lifted into the type system: each
//! `with_*` call flips one type-level flag from [`Omitted`] to
//! [`Fetched`], and the returned [`Policy`] has a real field where the
//! flag is `Fetched` and an inert [`NotFetched`] placeholder where it is
//! not. Reading data you did not request is a compile error, not an
//! `unwrap`.
//!
//! Callers whose selection is data rather than source code use
//! [`Client::get_policies_dynamic`](crate::Client::get_policies_dynamic),
//! which trades that guarantee back for `Option`s. Both getters share one
//! request path; only the response shaping differs.

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::BoxFuture;
use crate::client::Client;
use crate::error::{DecodeError, Error};
use crate::policy::model::{Category, PolicyEmployee, PolicyTags, ReportField, TaxConfig};
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

    /// The inverse of [`Wrap`](FetchState::Wrap): read a payload back out of
    /// its slot. [`Fetched`] yields `Some`, [`Omitted`] yields `None`.
    ///
    /// Code generic over the states cannot inspect a `Wrap<T>` — that is the
    /// point of the design — so this is the one way back down to a runtime
    /// shape, for a caller bridging a statically-typed [`Policy`] into a
    /// context that has to treat every section uniformly:
    ///
    /// ```
    /// use expensify::{FetchState, Fetched, NotFetched, Omitted};
    ///
    /// assert_eq!(Fetched::project::<u8>(7), Some(7));
    /// assert_eq!(Omitted::project::<u8>(NotFetched), None);
    /// ```
    ///
    /// [`Policy::project`] applies this to all five sections at once.
    fn project<T: Payload>(wrapped: Self::Wrap<T>) -> Option<T>;

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

    fn project<T: Payload>(wrapped: T) -> Option<T> {
        Some(wrapped)
    }

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

    fn project<T: Payload>(_wrapped: NotFetched) -> Option<T> {
        None
    }

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

/// A section of the policy response — the runtime spelling of the `with_*`
/// selections, for [`Client::get_policies_dynamic`](crate::Client::get_policies_dynamic).
///
/// Closed on purpose: these are the values this crate *sends*, and Expensify
/// rejects any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PolicyField {
    /// Expense categories.
    Categories,
    /// Report fields.
    ReportFields,
    /// Tags.
    Tags,
    /// The tax configuration.
    Tax,
    /// Policy members.
    Employees,
}

impl PolicyField {
    /// Spelling in `inputSettings.fields`; the response keys its sections
    /// with the same names.
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::Categories => "categories",
            Self::ReportFields => "reportFields",
            Self::Tags => "tags",
            Self::Tax => "tax",
            Self::Employees => "employees",
        }
    }
}

impl From<&PolicyField> for PolicyField {
    fn from(field: &PolicyField) -> Self {
        *field
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
    /// Tags, present when `with_tags()` was called. Either shape Expensify
    /// answers with; see [`PolicyTags`].
    pub tags: Tags::Wrap<PolicyTags>,
    /// `None` when the policy has no tax configuration (the API returns
    /// `"tax": {}`); this `Option` is data-dependent, not request-dependent.
    pub tax: Tax::Wrap<Option<TaxConfig>>,
    /// Policy members, present when `with_employees()` was called.
    pub employees: Emps::Wrap<Vec<PolicyEmployee>>,
}

/// Return type of an awaited [`GetPoliciesAction`], keyed by policy ID.
pub type Policies<Cats, Fields, Tags, Tax, Emps> =
    HashMap<PolicyId, Policy<Cats, Fields, Tags, Tax, Emps>>;

impl<Cats, Fields, Tags, Tax, Emps> Policy<Cats, Fields, Tags, Tax, Emps>
where
    Cats: FetchState,
    Fields: FetchState,
    Tags: FetchState,
    Tax: FetchState,
    Emps: FetchState,
{
    /// Erase the typestate: every section becomes an `Option`, `None` where
    /// it was not requested.
    ///
    /// This throws away the guarantee the type parameters carry, so it is for
    /// crossing into code that must handle all five sections uniformly —
    /// rendering, serializing, a plugin boundary. Reading a section straight
    /// off [`Policy`] needs no `unwrap` and no `None` arm; prefer that.
    #[must_use]
    pub fn project(self) -> DynamicPolicy {
        DynamicPolicy {
            categories: Cats::project(self.categories),
            report_fields: Fields::project(self.report_fields),
            tags: Tags::project(self.tags),
            tax: Tax::project(self.tax),
            employees: Emps::project(self.employees),
        }
    }
}

/// A policy whose sections are shaped at runtime: `Some` for a requested
/// section, `None` for one that was not.
///
/// Produced by [`Client::get_policies_dynamic`](crate::Client::get_policies_dynamic)
/// and by [`Policy::project`]. The `Option`s are the cost of not knowing the
/// selection at compile time — see the escape-hatch note on
/// `get_policies_dynamic`.
#[derive(Clone, Debug)]
pub struct DynamicPolicy {
    /// Expense categories, `Some` if [`PolicyField::Categories`] was requested.
    pub categories: Option<Vec<Category>>,
    /// Report fields, `Some` if [`PolicyField::ReportFields`] was requested.
    pub report_fields: Option<Vec<ReportField>>,
    /// Tags, `Some` if [`PolicyField::Tags`] was requested.
    pub tags: Option<PolicyTags>,
    /// Tax configuration. The outer `Option` is request-dependent; the inner
    /// one is data-dependent (`None` = the policy has no tax configuration),
    /// exactly the distinction [`Policy::tax`] keeps apart by construction.
    pub tax: Option<Option<TaxConfig>>,
    /// Policy members, `Some` if [`PolicyField::Employees`] was requested.
    pub employees: Option<Vec<PolicyEmployee>>,
}

/// Return type of an awaited [`GetPoliciesDynamicAction`], keyed by policy ID.
pub type DynamicPolicies = HashMap<PolicyId, DynamicPolicy>;

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
    fields: Vec<PolicyField>,
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
        field: PolicyField,
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
        self.cast(PolicyField::Categories)
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
        self.cast(PolicyField::ReportFields)
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
        self.cast(PolicyField::Tags)
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
        self.cast(PolicyField::Tax)
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
        self.cast(PolicyField::Employees)
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
            let raw = fetch(self.client, self.ids, self.fields, self.user_email).await?;

            let mut policies = Policies::new();
            for (id, sections) in raw {
                policies.insert(
                    id,
                    Policy {
                        categories: Cats::extract("categories", sections.categories)?,
                        report_fields: Fields::extract("reportFields", sections.report_fields)?,
                        tags: Tags::extract("tags", sections.tags)?,
                        tax: Tax::extract("tax", sections.tax)?,
                        employees: Emps::extract("employees", sections.employees)?,
                    },
                );
            }
            Ok(policies)
        })
    }
}

// ---- the dynamic escape hatch ---------------------------------------

/// Policy Getter with a runtime field selection. Awaits to
/// [`DynamicPolicies`].
///
/// See [`Client::get_policies_dynamic`](crate::Client::get_policies_dynamic)
/// for when this is the right getter — usually it is not.
#[must_use = "actions do nothing until awaited"]
pub struct GetPoliciesDynamicAction {
    client: Client,
    ids: Vec<PolicyId>,
    fields: Vec<PolicyField>,
    user_email: Option<String>,
}

impl GetPoliciesDynamicAction {
    pub(crate) fn new(client: Client, ids: Vec<PolicyId>, fields: Vec<PolicyField>) -> Self {
        // The static path cannot select a field twice (each `with_*` exists
        // only while its slot is `Omitted`); a `Vec` can, so dedupe here
        // rather than send `["tax","tax"]`.
        let mut deduped: Vec<PolicyField> = Vec::with_capacity(fields.len());
        for field in fields {
            if !deduped.contains(&field) {
                deduped.push(field);
            }
        }
        Self {
            client,
            ids,
            fields: deduped,
            user_email: None,
        }
    }

    /// See [`GetPoliciesBuilder::on_behalf_of`].
    pub fn on_behalf_of(mut self, email: impl Into<String>) -> Self {
        self.user_email = Some(email.into());
        self
    }
}

impl IntoFuture for GetPoliciesDynamicAction {
    type Output = Result<DynamicPolicies, Error>;
    type IntoFuture = BoxFuture<Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            let wanted = |field: PolicyField| self.fields.contains(&field);
            let (categories, report_fields, tags, tax, employees) = (
                wanted(PolicyField::Categories),
                wanted(PolicyField::ReportFields),
                wanted(PolicyField::Tags),
                wanted(PolicyField::Tax),
                wanted(PolicyField::Employees),
            );

            let raw = fetch(self.client, self.ids, self.fields, self.user_email).await?;

            let mut policies = DynamicPolicies::new();
            for (id, sections) in raw {
                policies.insert(
                    id,
                    DynamicPolicy {
                        categories: selected(categories, "categories", sections.categories)?,
                        report_fields: selected(
                            report_fields,
                            "reportFields",
                            sections.report_fields,
                        )?,
                        tags: selected(tags, "tags", sections.tags)?,
                        tax: selected(tax, "tax", sections.tax)?,
                        employees: selected(employees, "employees", sections.employees)?,
                    },
                );
            }
            Ok(policies)
        })
    }
}

/// Decode one section iff it was requested. Delegates to [`Fetched::extract`]
/// so a section the server left out of a response that asked for it is the
/// same error on both getters.
fn selected<T>(
    requested: bool,
    field: &'static str,
    value: Option<Value>,
) -> Result<Option<T>, Error>
where
    T: DeserializeOwned + Payload,
{
    if requested {
        Fetched::extract::<T>(field, value).map(Some)
    } else {
        Ok(None)
    }
}

// ---- shared request path --------------------------------------------

/// One policy's response object, split into sections but not yet decoded.
struct RawSections {
    categories: Option<Value>,
    report_fields: Option<Value>,
    tags: Option<Value>,
    tax: Option<Value>,
    employees: Option<Value>,
}

/// Everything both getters do identically: validate, send, split. The static
/// getter and the dynamic one differ only in how they decode the sections.
async fn fetch(
    client: Client,
    ids: Vec<PolicyId>,
    fields: Vec<PolicyField>,
    user_email: Option<String>,
) -> Result<Vec<(PolicyId, RawSections)>, Error> {
    if ids.is_empty() {
        return Err(Error::InvalidRequest(
            "get_policies needs at least one policy ID; \
             an empty policyIDList is a documented 410"
                .to_owned(),
        ));
    }
    // Unreachable from the static path, where `GetPoliciesBuilder` is the
    // type-level proof of one selection; reachable from the dynamic one.
    if fields.is_empty() {
        return Err(Error::InvalidRequest(
            "get_policies needs at least one field; \
             an empty fields list is a documented 410"
                .to_owned(),
        ));
    }

    let request = wire::get_policies(&ids, &fields, user_email.as_deref());
    let response = client.send(request).await?;

    Ok(wire::policy_info(response)?
        .into_iter()
        .map(|(id, mut info)| {
            let mut section = |key: &str| info.as_object_mut().and_then(|map| map.remove(key));
            let sections = RawSections {
                categories: section("categories"),
                report_fields: section("reportFields"),
                tags: section("tags"),
                tax: section("tax").map(wire::normalize_tax),
                employees: section("employees"),
            };
            (id, sections)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_inverts_wrap() {
        assert_eq!(Fetched::project::<u8>(7), Some(7));
        assert_eq!(Omitted::project::<u8>(NotFetched), None);
    }

    /// The reason `project` is on the trait: without it, code generic over
    /// the states can hold a `Wrap<T>` and never look inside.
    #[test]
    fn project_works_through_a_generic() {
        fn read<S: FetchState>(slot: S::Wrap<Vec<Category>>) -> usize {
            S::project(slot).map_or(0, |categories| categories.len())
        }

        assert_eq!(read::<Fetched>(vec![Category::new("Meals")]), 1);
        assert_eq!(read::<Omitted>(NotFetched), 0);
    }

    #[test]
    fn projecting_a_policy_keeps_the_requested_sections() {
        let policy: Policy<Fetched, Omitted, Omitted, Fetched, Omitted> = Policy {
            categories: vec![Category::new("Meals")],
            report_fields: NotFetched,
            tags: NotFetched,
            tax: None,
            employees: NotFetched,
        };

        let projected = policy.project();
        assert_eq!(projected.categories.as_deref().map(<[_]>::len), Some(1));
        assert!(projected.report_fields.is_none());
        assert!(projected.employees.is_none());
        // Requested but unconfigured: outer Some, inner None.
        assert_eq!(projected.tax, Some(None));
    }

    #[test]
    fn wire_names_match_the_static_selections() {
        assert_eq!(PolicyField::Categories.wire(), "categories");
        assert_eq!(PolicyField::ReportFields.wire(), "reportFields");
        assert_eq!(PolicyField::Tags.wire(), "tags");
        assert_eq!(PolicyField::Tax.wire(), "tax");
        assert_eq!(PolicyField::Employees.wire(), "employees");
    }
}
