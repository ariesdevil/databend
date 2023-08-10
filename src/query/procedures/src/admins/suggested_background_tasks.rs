// Copyright 2021 Datafuse Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use common_exception::Result;
use common_expression::types::DataType;
use common_expression::DataField;
use common_expression::DataSchema;
use common_expression::DataSchemaRefExt;

use crate::ProcedureFeatures;
use crate::ProcedureSignature;

pub struct SuggestedBackgroundTasksProcedureSig {}

impl SuggestedBackgroundTasksProcedureSig {
    pub fn try_create() -> Result<Box<dyn ProcedureSignature>> {
        Ok(Box::new(SuggestedBackgroundTasksProcedureSig {}))
    }
}

impl ProcedureSignature for SuggestedBackgroundTasksProcedureSig {
    fn name(&self) -> &str {
        "SUGGESTED_BACKGROUND_TASKS"
    }

    fn features(&self) -> ProcedureFeatures {
        ProcedureFeatures::default()
            .variadic_arguments(0, 1)
            .management_mode_required(false)
    }

    fn schema(&self) -> Arc<DataSchema> {
        DataSchemaRefExt::create(vec![
            DataField::new("type", DataType::String),
            DataField::new("database_name", DataType::String),
            DataField::new("table_name", DataType::String),
            DataField::new("table_statistics", DataType::Variant),
            DataField::new("should_do_segment_compact", DataType::Boolean),
            DataField::new("should_do_compact", DataType::Boolean),
        ])
    }
}
