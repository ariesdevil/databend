// Copyright 2020-2021 The Datafuse Authors.
//
// SPDX-License-Identifier: Apache-2.0.

use std::any::Any;
use std::sync::Arc;

use common_datablocks::DataBlock;
use common_datavalues::DataField;
use common_datavalues::DataSchemaRef;
use common_datavalues::DataSchemaRefExt;
use common_datavalues::DataType;
use common_datavalues::StringArray;
use common_exception::Result;
use common_planners::Partition;
use common_planners::ReadDataSourcePlan;
use common_planners::ScanPlan;
use common_planners::Statistics;
use common_streams::DataBlockStream;
use common_streams::SendableDataBlockStream;

use crate::datasources::ITable;
use crate::sessions::FuseQueryContextRef;

pub struct ContributorsTable {
    schema: DataSchemaRef
}

impl ContributorsTable {
    pub fn create() -> Self {
        ContributorsTable {
            schema: DataSchemaRefExt::create(vec![DataField::new("name", DataType::Utf8, false)])
        }
    }
}

#[async_trait::async_trait]
impl ITable for ContributorsTable {
    fn name(&self) -> &str {
        "contributors"
    }

    fn engine(&self) -> &str {
        "SystemContributors"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> Result<DataSchemaRef> {
        Ok(self.schema.clone())
    }

    fn is_local(&self) -> bool {
        true
    }

    fn read_plan(&self, _ctx: FuseQueryContextRef, _scan: &ScanPlan) -> Result<ReadDataSourcePlan> {
        Ok(ReadDataSourcePlan {
            db: "system".to_string(),
            table: self.name().to_string(),
            schema: self.schema.clone(),
            partitions: vec![Partition {
                name: "".to_string(),
                version: 0
            }],
            statistics: Statistics::default(),
            description: "(Read from system.contributors table)".to_string()
        })
    }

    async fn read(&self, _ctx: FuseQueryContextRef) -> Result<SendableDataBlockStream> {
        let contributors: Vec<&str> = env!("COMMIT_AUTHORS")
            .split_terminator(',')
            .map(|x| x.trim())
            .collect();
        let block = DataBlock::create_by_array(self.schema.clone(), vec![Arc::new(
            StringArray::from(contributors)
        )]);
        Ok(Box::pin(DataBlockStream::create(
            self.schema.clone(),
            None,
            vec![block]
        )))
    }
}
