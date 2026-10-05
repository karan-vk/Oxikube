//! Serde types for the `meta.k8s.io/v1` Table wire format.
//!
//! kube-rs has no Table support, so these are our own (ADR 0006), shaped after the apiserver's
//! `metav1.Table` as kubetui and sofka read it (research 2.3a). Every field is defaulted so a
//! response that is *not* a Table (the server ignored the `Accept` header) still decodes; its
//! `kind` then names the list type (`PodList`) and its objects arrive in `items`.

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// The `kind` of a Table response.
pub(crate) const TABLE_KIND: &str = "Table";

/// A Table response, or the plain list the server sent instead of one.
///
/// No `Debug`: in the fallback `items` holds whole objects, which may be Secrets.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Table {
    /// `Table` when the server honoured the Accept header, else the list kind.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) kind: String,
    /// Collection metadata. On a Table watch event, `resourceVersion` is the object's.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) metadata: ListMeta,
    /// Column definitions. On a Table watch only the first event of a connection has them;
    /// the others send `null`.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) column_definitions: Vec<ColumnDefinition>,
    /// The rows (Table only).
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) rows: Vec<TableRow>,
    /// The objects of a plain list (fallback only).
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) items: Vec<Value>,
}

impl Table {
    /// Whether the server answered with a Table.
    pub(crate) fn is_table(&self) -> bool {
        self.kind == TABLE_KIND
    }
}

/// `metav1.ListMeta`: the paging and version fields.
#[derive(Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListMeta {
    /// The collection's resource version.
    #[serde(default)]
    pub(crate) resource_version: Option<String>,
    /// Continue token for the next page; empty or absent on the last page.
    #[serde(default, rename = "continue")]
    pub(crate) continue_token: Option<String>,
}

/// `metav1.TableColumnDefinition`.
#[derive(Deserialize, Default, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ColumnDefinition {
    /// Header text.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) name: String,
    /// OpenAPI type (`string`, `integer`, `number`, `boolean`, `date`).
    #[serde(default, rename = "type", deserialize_with = "null_as_default")]
    pub(crate) column_type: String,
    /// OpenAPI format hint (`name`, `date-time`, `quantity`...).
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) format: String,
    /// Human description.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) description: String,
    /// `0` default view, higher only in `-o wide`.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) priority: i32,
}

/// `metav1.TableRow`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableRow {
    /// One value per column.
    #[serde(default, deserialize_with = "null_as_default")]
    pub(crate) cells: Vec<Value>,
    /// The embedded object: `PartialObjectMetadata` with `includeObject=Metadata`, the whole
    /// object with `Object`, absent with `None`.
    #[serde(default)]
    pub(crate) object: Option<Value>,
}

/// `null` as the type's default: the apiserver writes `null` for empty slices (Go `nil`),
/// for example `columnDefinitions` on every Table watch event after the first.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}
