wit_bindgen::generate!({
    path: "../../wit",
    world: "gateway-component",
});

use exports::ehlung::commerce::gateway::{Guest, RouteAction, RouteDecision, RouteTarget};

struct GatewayComponent;

impl Guest for GatewayComponent {
    fn resolve(method: String, path: String) -> RouteDecision {
        let method = method.to_ascii_uppercase();
        let segments: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .filter(|v| !v.is_empty())
            .collect();

        match (method.as_str(), segments.as_slice()) {
            ("GET", ["health"]) => decision(RouteTarget::Health, RouteAction::Health, None),
            ("GET", ["api", "products"]) => decision(RouteTarget::Product, RouteAction::List, None),
            ("POST", ["api", "products"]) => {
                decision(RouteTarget::Product, RouteAction::Create, None)
            }
            ("PATCH", ["api", "products", id, "stock"]) => id
                .parse()
                .ok()
                .map(|id| decision(RouteTarget::Inventory, RouteAction::Adjust, Some(id)))
                .unwrap_or_else(not_found),
            ("POST", ["api", "orders"]) => decision(RouteTarget::Order, RouteAction::Create, None),
            ("GET", ["api", "orders", id]) => id
                .parse()
                .ok()
                .map(|id| decision(RouteTarget::Order, RouteAction::Get, Some(id)))
                .unwrap_or_else(not_found),
            ("POST", ["api", "orders", id, "cancel"]) => id
                .parse()
                .ok()
                .map(|id| decision(RouteTarget::Order, RouteAction::Cancel, Some(id)))
                .unwrap_or_else(not_found),
            _ => not_found(),
        }
    }
}

fn decision(target: RouteTarget, action: RouteAction, resource_id: Option<u64>) -> RouteDecision {
    RouteDecision {
        target,
        action,
        resource_id,
    }
}

fn not_found() -> RouteDecision {
    decision(RouteTarget::NotFound, RouteAction::NotFound, None)
}

export!(GatewayComponent);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_product_stock_route() {
        let result = GatewayComponent::resolve("PATCH".into(), "/api/products/42/stock".into());
        assert_eq!(result.resource_id, Some(42));
        assert!(matches!(result.target, RouteTarget::Inventory));
    }

    #[test]
    fn rejects_unknown_route() {
        let result = GatewayComponent::resolve("GET".into(), "/unknown".into());
        assert!(matches!(result.target, RouteTarget::NotFound));
    }
}
