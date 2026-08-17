wit_bindgen::generate!({
    path: "../../wit",
    world: "order-component",
});

use std::collections::BTreeSet;

use ehlung::commerce::{
    models::{Order, OrderInput},
    order_repository,
};
use exports::ehlung::commerce::order_agent::Guest;

struct OrderAgent;

impl Guest for OrderAgent {
    fn create(input: OrderInput) -> Result<Order, String> {
        validate_input(&input)?;
        let quote = order_repository::quote(&input)?;

        if quote.lines.len() != input.lines.len() {
            return Err("주문 상품 견적 결과가 요청과 일치하지 않습니다.".to_string());
        }
        for line in &quote.lines {
            if u64::from(line.quantity) > line.available_stock {
                return Err(format!("상품 {}의 재고가 부족합니다.", line.product_id));
            }
        }

        order_repository::create(&input, quote.total_cents)
    }

    fn get(order_id: u64) -> Result<Option<Order>, String> {
        if order_id == 0 {
            return Err("주문 ID는 0보다 커야 합니다.".to_string());
        }
        order_repository::get(order_id)
    }

    fn cancel(order_id: u64) -> Result<Order, String> {
        if order_id == 0 {
            return Err("주문 ID는 0보다 커야 합니다.".to_string());
        }
        order_repository::cancel(order_id)
    }
}

fn validate_input(input: &OrderInput) -> Result<(), String> {
    if input.lines.is_empty() {
        return Err("주문에는 한 개 이상의 상품이 필요합니다.".to_string());
    }
    if input.lines.len() > 50 {
        return Err("한 주문에는 최대 50개 상품만 포함할 수 있습니다.".to_string());
    }

    let mut seen = BTreeSet::new();
    for line in &input.lines {
        if line.product_id == 0 {
            return Err("상품 ID는 0보다 커야 합니다.".to_string());
        }
        if line.quantity == 0 || line.quantity > 100 {
            return Err("상품별 주문 수량은 1~100개여야 합니다.".to_string());
        }
        if !seen.insert(line.product_id) {
            return Err(format!("상품 {}가 주문에 중복되었습니다.", line.product_id));
        }
    }
    Ok(())
}

export!(OrderAgent);

#[cfg(test)]
mod tests {
    use super::*;
    use ehlung::commerce::models::OrderLineInput;

    #[test]
    fn rejects_duplicate_products() {
        let input = OrderInput {
            lines: vec![
                OrderLineInput {
                    product_id: 1,
                    quantity: 1,
                },
                OrderLineInput {
                    product_id: 1,
                    quantity: 2,
                },
            ],
        };
        assert!(validate_input(&input).is_err());
    }

    #[test]
    fn accepts_valid_order() {
        let input = OrderInput {
            lines: vec![OrderLineInput {
                product_id: 1,
                quantity: 2,
            }],
        };
        assert!(validate_input(&input).is_ok());
    }
}
