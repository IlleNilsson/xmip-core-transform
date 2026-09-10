#![forbid(unsafe_code)]

use contract::{ContractDescriptor, StructureReader};
use message::Message;
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct TransformError {
    pub message: String,
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for TransformError {}

pub struct TransformRequest<'a> {
    pub message: &'a Message,
    pub input: &'a dyn StructureReader,
    pub output_contract: &'a ContractDescriptor,
}

pub trait Transformer: Send + Sync {
    fn name(&self) -> &'static str;
    fn input_contract(&self) -> &ContractDescriptor;
    fn output_contract(&self) -> &ContractDescriptor;
    fn transform(&self, request: TransformRequest<'_>) -> Result<Message, TransformError>;
}

pub trait TransformRegistry: Send + Sync {
    fn resolve(
        &self,
        input: &ContractDescriptor,
        output: &ContractDescriptor,
        name: Option<&str>,
    ) -> Option<&dyn Transformer>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use contract::ContractId;

    fn descriptor(id: &str, representation: &str) -> ContractDescriptor {
        ContractDescriptor {
            id: ContractId(id.to_string()),
            version: "1".to_string(),
            representation: representation.to_string(),
        }
    }

    /// A transformer that names its two contracts and nothing more; the
    /// registry is what is under test.
    struct Named {
        name: &'static str,
        input: ContractDescriptor,
        output: ContractDescriptor,
    }

    impl Transformer for Named {
        fn name(&self) -> &'static str {
            self.name
        }

        fn input_contract(&self) -> &ContractDescriptor {
            &self.input
        }

        fn output_contract(&self) -> &ContractDescriptor {
            &self.output
        }

        fn transform(&self, _: TransformRequest<'_>) -> Result<Message, TransformError> {
            Err(TransformError {
                message: "not exercised here".to_string(),
            })
        }
    }

    struct Registry(Vec<Named>);

    impl TransformRegistry for Registry {
        fn resolve(
            &self,
            input: &ContractDescriptor,
            output: &ContractDescriptor,
            name: Option<&str>,
        ) -> Option<&dyn Transformer> {
            self.0
                .iter()
                .find(|t| {
                    t.input_contract() == input
                        && t.output_contract() == output
                        && name.is_none_or(|n| n == t.name())
                })
                .map(|t| t as &dyn Transformer)
        }
    }

    #[test]
    fn a_registry_resolves_by_contracts_and_optionally_by_name() {
        let registry = Registry(vec![
            Named {
                name: "order-to-invoice",
                input: descriptor("order", "application/json"),
                output: descriptor("invoice", "application/xml"),
            },
            Named {
                name: "order-to-invoice-v2",
                input: descriptor("order", "application/json"),
                output: descriptor("invoice", "application/xml"),
            },
        ]);
        let order = descriptor("order", "application/json");
        let invoice = descriptor("invoice", "application/xml");
        assert_eq!(
            registry
                .resolve(&order, &invoice, None)
                .map(Transformer::name),
            Some("order-to-invoice")
        );
        assert_eq!(
            registry
                .resolve(&order, &invoice, Some("order-to-invoice-v2"))
                .map(Transformer::name),
            Some("order-to-invoice-v2")
        );
        assert!(registry.resolve(&invoice, &order, None).is_none());
        assert!(registry.resolve(&order, &invoice, Some("other")).is_none());
    }

    #[test]
    fn a_transform_error_says_why() {
        let error = TransformError {
            message: "no mapping for line 3".to_string(),
        };
        assert_eq!(error.to_string(), "no mapping for line 3");
    }
}
