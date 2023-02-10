//  Copyright 2021 Datafuse Labs.
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.

use std::collections::HashMap;
use std::sync::Arc;

use common_exception::Result;
use common_expression::BlockThresholds;
use common_expression::ColumnId;
use common_expression::DataBlock;
use storages_common_table_meta::meta;
use storages_common_table_meta::meta::BlockMeta;
use storages_common_table_meta::meta::ColumnMeta;
use storages_common_table_meta::meta::Location;
use storages_common_table_meta::meta::Statistics;
use storages_common_table_meta::meta::StatisticsOfColumns;
use storages_common_table_meta::meta::Versioned;

use crate::statistics::block_statistics::BlockStatistics;

#[derive(Default, Clone)]
pub struct BlockMetaStats {
    pub block_metas: Vec<Arc<BlockMeta>>,
    pub stats: Statistics,
    pub blocks_statistics: Vec<StatisticsOfColumns>,
}

impl BlockMetaStats {
    pub fn summary(&self) -> Result<StatisticsOfColumns> {
        super::reduce_block_statistics(&self.blocks_statistics, None)
    }
}

#[derive(Default)]
pub struct StatisticsAccumulator {
    pub blocks_metas: Vec<Arc<BlockMeta>>,
    pub block_meta_map: HashMap<String, BlockMetaStats>,
    pub blocks_statistics: Vec<StatisticsOfColumns>,
    pub summary_row_count: u64,
    pub summary_block_count: u64,
    pub in_memory_size: u64,
    pub file_size: u64,
    pub index_size: u64,

    pub perfect_block_count: u64,
    pub thresholds: BlockThresholds,
}

impl StatisticsAccumulator {
    pub fn new(thresholds: BlockThresholds) -> Self {
        Self {
            thresholds,
            ..Default::default()
        }
    }

    pub fn add_block(
        &mut self,
        file_size: u64,
        col_metas: HashMap<ColumnId, ColumnMeta>,
        block_statistics: BlockStatistics,
        bloom_filter_index_location: Option<Location>,
        bloom_filter_index_size: u64,
        block_compression: meta::Compression,
        mode: String,
    ) -> Result<()> {
        self.add(
            file_size,
            col_metas,
            block_statistics,
            bloom_filter_index_location,
            bloom_filter_index_size,
            block_compression,
            mode,
        )
    }

    pub fn add_with_block_meta(
        &mut self,
        block_meta: BlockMeta,
        block_statistics: BlockStatistics,
        block_compression: meta::Compression,
    ) -> Result<()> {
        let bloom_filter_index_location = block_meta.bloom_filter_index_location;
        let bloom_filter_index_size = block_meta.bloom_filter_index_size;
        let file_size = block_meta.file_size;
        let col_metas = block_meta.col_metas;

        self.add(
            file_size,
            col_metas,
            block_statistics,
            bloom_filter_index_location,
            bloom_filter_index_size,
            block_compression,
            "skipfile".to_string(),
        )
    }

    pub fn summary(&self) -> Result<StatisticsOfColumns> {
        super::reduce_block_statistics(&self.blocks_statistics, None, None)
    }

    fn add(
        &mut self,
        file_size: u64,
        column_meta: HashMap<ColumnId, ColumnMeta>,
        block_statistics: BlockStatistics,
        bloom_filter_index_location: Option<Location>,
        bloom_filter_index_size: u64,
        block_compression: meta::Compression,
        mode: String,
    ) -> Result<()> {
        // row_count: acc.summary_row_count,
        // block_count: acc.summary_block_count,
        // perfect_block_count: acc.perfect_block_count,
        // uncompressed_byte_size: acc.in_memory_size,
        // compressed_byte_size: acc.file_size,
        // index_size: acc.index_size,
        // col_stats,
        self.file_size += file_size;
        self.index_size += bloom_filter_index_size;
        self.summary_block_count += 1;
        self.in_memory_size += block_statistics.block_bytes_size;
        self.summary_row_count += block_statistics.block_rows_size;
        self.blocks_statistics
            .push(block_statistics.block_column_statistics.clone());

        let row_count = block_statistics.block_rows_size;
        let block_size = block_statistics.block_bytes_size;
        let col_stats = block_statistics.block_column_statistics.clone();
        let data_location = (block_statistics.block_file_location, DataBlock::VERSION);
        let cluster_stats = block_statistics.block_cluster_statistics;
        let belong_to = block_statistics.block_belong_to;

        if self
            .thresholds
            .check_large_enough(row_count as usize, block_size as usize)
        {
            self.perfect_block_count += 1;
        }

        if mode == "skipfile".to_string() {
            let block_meta = Arc::new(BlockMeta::new(
                row_count,
                block_size,
                file_size,
                col_stats,
                column_meta,
                cluster_stats,
                data_location,
                bloom_filter_index_location,
                bloom_filter_index_size,
                block_compression,
                belong_to.clone(),
            ));

            self.block_meta_map
                .entry(belong_to.unwrap())
                .and_modify(|block_metas| {
                    block_metas.block_metas.push(block_meta.clone());
                    block_metas.stats.row_count += row_count;
                    block_metas.stats.block_count += 1;
                    if self
                        .thresholds
                        .check_large_enough(row_count as usize, block_size as usize)
                    {
                        block_metas.stats.perfect_block_count += 1;
                    }
                    block_metas.stats.index_size += bloom_filter_index_size;
                    block_metas.stats.compressed_byte_size += file_size;
                    block_metas.stats.uncompressed_byte_size += block_statistics.block_bytes_size;
                    block_metas
                        .blocks_statistics
                        .push(block_statistics.block_column_statistics.clone());
                })
                .or_insert({
                    let mut block_metas = BlockMetaStats::default();
                    block_metas.block_metas.push(block_meta);
                    block_metas.stats.row_count += row_count;
                    block_metas.stats.block_count += 1;
                    if self
                        .thresholds
                        .check_large_enough(row_count as usize, block_size as usize)
                    {
                        block_metas.stats.perfect_block_count += 1;
                    }
                    block_metas.stats.index_size += bloom_filter_index_size;
                    block_metas.stats.compressed_byte_size += file_size;
                    block_metas.stats.uncompressed_byte_size += block_statistics.block_bytes_size;
                    block_metas
                        .blocks_statistics
                        .push(block_statistics.block_column_statistics.clone());

                    block_metas
                });
        } else {
            self.blocks_metas.push(Arc::new(BlockMeta::new(
                row_count,
                block_size,
                file_size,
                col_stats,
                column_meta,
                cluster_stats,
                data_location,
                bloom_filter_index_location,
                bloom_filter_index_size,
                block_compression,
                belong_to,
            )));
        }

        Ok(())
    }
}
