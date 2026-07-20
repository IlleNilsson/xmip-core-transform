#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use xmip_contract::{ContractDescriptor, StructureReader};
use xmip_message::Message;

#[derive(Debug)]
pub struct TransformError {
    pub message: String,
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(&self.message) }
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
