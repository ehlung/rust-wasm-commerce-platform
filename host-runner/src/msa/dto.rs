use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductDto {
    pub id: u64,
    pub sku: String,
    pub name: String,
    pub price_cents: u64,
    pub stock: u64,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProductDto {
    pub sku: String,
    pub name: String,
    pub price_cents: u64,
    pub initial_stock: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AdjustStockDto {
    pub delta: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderLineInputDto {
    pub product_id: u64,
    pub quantity: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateOrderDto {
    pub lines: Vec<OrderLineInputDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteLineDto {
    pub product_id: u64,
    pub sku: String,
    pub product_name: String,
    pub quantity: u32,
    pub unit_price_cents: u64,
    pub available_stock: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteDto {
    pub lines: Vec<QuoteLineDto>,
    pub total_cents: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReservationDto {
    pub token: String,
    pub lines: Vec<OrderLineInputDto>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderLineDto {
    pub product_id: u64,
    pub sku: String,
    pub product_name: String,
    pub quantity: u32,
    pub unit_price_cents: u64,
    pub line_total_cents: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderDto {
    pub id: u64,
    pub status: String,
    pub total_cents: u64,
    pub lines: Vec<OrderLineDto>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorBody {
    pub error: String,
}
