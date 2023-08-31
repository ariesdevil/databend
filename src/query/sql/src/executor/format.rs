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

use common_ast::ast::FormatTreeNode;
use common_catalog::plan::PartStatistics;
use common_exception::Result;
use common_expression::DataSchemaRef;
use common_functions::BUILTIN_FUNCTIONS;
use common_profile::SharedProcessorProfiles;
use itertools::Itertools;

use super::AggregateExpand;
use super::AggregateFinal;
use super::AggregateFunctionDesc;
use super::AggregatePartial;
use super::CopyIntoTable;
use super::DeletePartial;
use super::EvalScalar;
use super::Exchange;
use super::Filter;
use super::HashJoin;
use super::Lambda;
use super::Limit;
use super::MutationAggregate;
use super::PhysicalPlan;
use super::Project;
use super::ProjectSet;
use super::RowFetch;
use super::Sort;
use super::TableScan;
use super::UnionAll;
use super::WindowFunction;
use crate::executor::explain::PlanStatsInfo;
use crate::executor::ConstantTableScan;
use crate::executor::CteScan;
use crate::executor::DistributedInsertSelect;
use crate::executor::ExchangeSink;
use crate::executor::ExchangeSource;
use crate::executor::FragmentKind;
use crate::executor::MaterializedCte;
use crate::executor::RangeJoin;
use crate::executor::RangeJoinType;
use crate::executor::RuntimeFilterSource;
use crate::executor::Window;
use crate::planner::Metadata;
use crate::planner::MetadataRef;
use crate::planner::DUMMY_TABLE_INDEX;

impl PhysicalPlan {
    pub fn format(
        &self,
        metadata: MetadataRef,
        prof_span_set: SharedProcessorProfiles,
    ) -> Result<FormatTreeNode<String>> {
        let metadata = metadata.read().clone();
        to_format_tree(self, &metadata, &prof_span_set)
    }

    pub fn format_join(&self, metadata: &MetadataRef) -> Result<FormatTreeNode<String>> {
        match self {
            PhysicalPlan::TableScan(plan) => {
                if plan.table_index == DUMMY_TABLE_INDEX {
                    return Ok(FormatTreeNode::with_children(
                        format!("Scan: dummy, rows: {}", plan.source.statistics.read_rows),
                        vec![],
                    ));
                }
                let table = metadata.read().table(plan.table_index).clone();
                let table_name =
                    format!("{}.{}.{}", table.catalog(), table.database(), table.name());

                Ok(FormatTreeNode::with_children(
                    format!(
                        "Scan: {} (read rows: {})",
                        table_name, plan.source.statistics.read_rows
                    ),
                    vec![],
                ))
            }
            PhysicalPlan::HashJoin(plan) => {
                let build_child = plan.build.format_join(metadata)?;
                let probe_child = plan.probe.format_join(metadata)?;

                let children = vec![
                    FormatTreeNode::with_children("Build".to_string(), vec![build_child]),
                    FormatTreeNode::with_children("Probe".to_string(), vec![probe_child]),
                ];

                let estimated_rows = if let Some(info) = &plan.stat_info {
                    format!("{0:.2}", info.estimated_rows)
                } else {
                    String::from("None")
                };

                Ok(FormatTreeNode::with_children(
                    format!("HashJoin: {} (rows: {})", plan.join_type, estimated_rows),
                    children,
                ))
            }
            PhysicalPlan::RangeJoin(plan) => {
                let left_child = plan.left.format_join(metadata)?;
                let right_child = plan.right.format_join(metadata)?;

                let children = vec![
                    FormatTreeNode::with_children("Left".to_string(), vec![left_child]),
                    FormatTreeNode::with_children("Right".to_string(), vec![right_child]),
                ];

                let estimated_rows = if let Some(info) = &plan.stat_info {
                    format!("{0:.2}", info.estimated_rows)
                } else {
                    String::from("none")
                };

                Ok(FormatTreeNode::with_children(
                    format!("RangeJoin: {} (rows: {})", plan.join_type, estimated_rows),
                    children,
                ))
            }
            PhysicalPlan::CteScan(cte_scan) => Ok(FormatTreeNode::with_children(
                format!(
                    "CteScan: {}, sub index: {}",
                    cte_scan.cte_idx.0, cte_scan.cte_idx.1
                ),
                vec![],
            )),
            PhysicalPlan::MaterializedCte(materialized_cte) => {
                let left_child = materialized_cte.left.format_join(metadata)?;
                let right_child = materialized_cte.right.format_join(metadata)?;
                let children = vec![
                    FormatTreeNode::with_children("Left".to_string(), vec![left_child]),
                    FormatTreeNode::with_children("Right".to_string(), vec![right_child]),
                ];
                Ok(FormatTreeNode::with_children(
                    format!("MaterializedCte: {}", materialized_cte.cte_idx),
                    children,
                ))
            }
            other => {
                let children = other
                    .children()
                    .map(|child| child.format_join(metadata))
                    .collect::<Result<Vec<FormatTreeNode<String>>>>()?;

                if children.len() == 1 {
                    Ok(children[0].clone())
                } else {
                    Ok(FormatTreeNode::with_children(
                        format!("{:?}", other),
                        children,
                    ))
                }
            }
        }
    }
}

fn to_format_tree(
    plan: &PhysicalPlan,
    metadata: &Metadata,
    profs: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    match plan {
        PhysicalPlan::TableScan(plan) => table_scan_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Filter(plan) => filter_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Project(plan) => project_to_format_tree(plan, metadata, profs),
        PhysicalPlan::EvalScalar(plan) => eval_scalar_to_format_tree(plan, metadata, profs),
        PhysicalPlan::AggregateExpand(plan) => {
            aggregate_expand_to_format_tree(plan, metadata, profs)
        }
        PhysicalPlan::AggregatePartial(plan) => {
            aggregate_partial_to_format_tree(plan, metadata, profs)
        }
        PhysicalPlan::AggregateFinal(plan) => aggregate_final_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Window(plan) => window_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Sort(plan) => sort_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Limit(plan) => limit_to_format_tree(plan, metadata, profs),
        PhysicalPlan::RowFetch(plan) => row_fetch_to_format_tree(plan, metadata, profs),
        PhysicalPlan::HashJoin(plan) => hash_join_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Exchange(plan) => exchange_to_format_tree(plan, metadata, profs),
        PhysicalPlan::UnionAll(plan) => union_all_to_format_tree(plan, metadata, profs),
        PhysicalPlan::ExchangeSource(plan) => exchange_source_to_format_tree(plan, metadata),
        PhysicalPlan::ExchangeSink(plan) => exchange_sink_to_format_tree(plan, metadata, profs),
        PhysicalPlan::DistributedInsertSelect(plan) => {
            distributed_insert_to_format_tree(plan.as_ref(), metadata, profs)
        }
        PhysicalPlan::DeletePartial(plan) => {
            delete_partial_to_format_tree(plan.as_ref(), metadata, profs)
        }
        PhysicalPlan::MutationAggregate(plan) => {
            delete_final_to_format_tree(plan.as_ref(), metadata, profs)
        }
        PhysicalPlan::ProjectSet(plan) => project_set_to_format_tree(plan, metadata, profs),
        PhysicalPlan::Lambda(plan) => lambda_to_format_tree(plan, metadata, profs),
        PhysicalPlan::RuntimeFilterSource(plan) => {
            runtime_filter_source_to_format_tree(plan, metadata, profs)
        }
        PhysicalPlan::RangeJoin(plan) => range_join_to_format_tree(plan, metadata, profs),
        PhysicalPlan::CopyIntoTable(plan) => copy_into_table(plan),
        PhysicalPlan::AsyncSourcer(_) => Ok(FormatTreeNode::new("AsyncSourcer".to_string())),
        PhysicalPlan::Deduplicate(_) => Ok(FormatTreeNode::new("Deduplicate".to_string())),
        PhysicalPlan::ReplaceInto(_) => Ok(FormatTreeNode::new("Replace".to_string())),
        PhysicalPlan::MergeInto(_) => Ok(FormatTreeNode::new("MergeInto".to_string())),
        PhysicalPlan::MergeIntoSource(_) => Ok(FormatTreeNode::new("MergeIntoSource".to_string())),
        PhysicalPlan::CteScan(plan) => cte_scan_to_format_tree(plan),
        PhysicalPlan::MaterializedCte(plan) => {
            materialized_cte_to_format_tree(plan, metadata, profs)
        }
        PhysicalPlan::ConstantTableScan(plan) => constant_table_scan_to_format_tree(plan, metadata),
        PhysicalPlan::FinalCommit(_) => Ok(FormatTreeNode::new("FinalCommit".to_string())),
    }
}

/// Helper function to add profile info to the format tree.
fn append_profile_info(
    children: &mut Vec<FormatTreeNode<String>>,
    profs: &SharedProcessorProfiles,
    plan_id: u32,
) {
    if let Some(prof) = profs.lock().unwrap().get(&plan_id) {
        children.push(FormatTreeNode::new(format!(
            "output rows: {}",
            prof.output_rows,
        )));
        children.push(FormatTreeNode::new(format!(
            "output bytes: {}",
            prof.output_bytes,
        )));
        children.push(FormatTreeNode::new(format!(
            "total cpu time: {:.3}ms",
            prof.cpu_time.as_secs_f64() * 1000.0
        )));
        children.push(FormatTreeNode::new(format!(
            "total wait time: {:.3}ms",
            prof.wait_time.as_secs_f64() * 1000.0
        )));
    }
}

fn copy_into_table(plan: &CopyIntoTable) -> Result<FormatTreeNode<String>> {
    Ok(FormatTreeNode::new(format!(
        "CopyIntoTable: {}",
        plan.table_info
    )))
}

fn table_scan_to_format_tree(
    plan: &TableScan,
    metadata: &Metadata,
    profs: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    if plan.table_index == DUMMY_TABLE_INDEX {
        return Ok(FormatTreeNode::new("DummyTableScan".to_string()));
    }
    let table = metadata.table(plan.table_index).clone();
    let table_name = format!("{}.{}.{}", table.catalog(), table.database(), table.name());
    let filters = plan
        .source
        .push_downs
        .as_ref()
        .and_then(|extras| {
            extras
                .filter
                .as_ref()
                .map(|expr| expr.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        })
        .unwrap_or_default();

    let limit = plan
        .source
        .push_downs
        .as_ref()
        .map_or("NONE".to_string(), |extras| {
            extras
                .limit
                .map_or("NONE".to_string(), |limit| limit.to_string())
        });

    let virtual_columns = plan.source.push_downs.as_ref().and_then(|extras| {
        extras.virtual_columns.as_ref().map(|columns| {
            let mut names = columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
            names.sort();
            names.iter().join(", ")
        })
    });

    let agg_index = plan
        .source
        .push_downs
        .as_ref()
        .and_then(|extras| extras.agg_index.as_ref());

    let mut children = vec![
        FormatTreeNode::new(format!("table: {table_name}")),
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, false)
        )),
    ];

    // Part stats.
    children.extend(part_stats_info_to_format_tree(&plan.source.statistics));
    // Push downs.
    let push_downs = match virtual_columns {
        Some(virtual_columns) => {
            format!(
                "push downs: [filters: [{filters}], limit: {limit}, virtual_columns: [{virtual_columns}]]"
            )
        }
        None => {
            format!("push downs: [filters: [{filters}], limit: {limit}]")
        }
    };
    children.push(FormatTreeNode::new(push_downs));
    // Aggregating index
    if let Some(agg_index) = agg_index {
        let (_, agg_index_sql, _) = metadata
            .get_agg_indexes(&table_name)
            .unwrap()
            .iter()
            .find(|(index, _, _)| *index == agg_index.index_id)
            .unwrap();

        children.push(FormatTreeNode::new(format!(
            "aggregating index: [{agg_index_sql}]"
        )));

        let agg_sel = agg_index
            .selection
            .iter()
            .map(|(expr, _)| expr.as_expr(&BUILTIN_FUNCTIONS).sql_display())
            .join(", ");
        let agg_filter = agg_index
            .filter
            .as_ref()
            .map(|f| f.as_expr(&BUILTIN_FUNCTIONS).sql_display());
        let text = if let Some(f) = agg_filter {
            format!("rewritten query: [selection: [{agg_sel}], filter: {f}]")
        } else {
            format!("rewritten query: [selection: [{agg_sel}]]")
        };
        children.push(FormatTreeNode::new(text));
    }

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, profs, plan.plan_id);

    Ok(FormatTreeNode::with_children(
        "TableScan".to_string(),
        children,
    ))
}

fn cte_scan_to_format_tree(plan: &CteScan) -> Result<FormatTreeNode<String>> {
    let cte_idx = FormatTreeNode::new(format!(
        "CTE index: {}, sub index: {}",
        plan.cte_idx.0, plan.cte_idx.1
    ));
    Ok(FormatTreeNode::with_children("CTEScan".to_string(), vec![
        cte_idx,
    ]))
}

fn constant_table_scan_to_format_tree(
    plan: &ConstantTableScan,
    metadata: &Metadata,
) -> Result<FormatTreeNode<String>> {
    let mut children = Vec::with_capacity(plan.values.len() + 1);
    children.push(FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    )));
    for (i, value) in plan.values.iter().enumerate() {
        let column = value.iter().map(|val| format!("{val}")).join(", ");
        children.push(FormatTreeNode::new(format!("column {}: [{}]", i, column)));
    }
    Ok(FormatTreeNode::with_children(
        "ConstantTableScan".to_string(),
        children,
    ))
}

fn filter_to_format_tree(
    plan: &Filter,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let filter = plan
        .predicates
        .iter()
        .map(|pred| pred.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        .join(", ");
    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("filters: [{filter}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "Filter".to_string(),
        children,
    ))
}

fn project_to_format_tree(
    plan: &Project,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    ))];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "Project".to_string(),
        children,
    ))
}

fn eval_scalar_to_format_tree(
    plan: &EvalScalar,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    if plan.exprs.is_empty() {
        return to_format_tree(&plan.input, metadata, prof_span_set);
    }
    let scalars = plan
        .exprs
        .iter()
        .map(|(expr, _)| expr.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        .join(", ");
    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("expressions: [{scalars}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "EvalScalar".to_string(),
        children,
    ))
}

pub fn pretty_display_agg_desc(desc: &AggregateFunctionDesc, metadata: &Metadata) -> String {
    format!(
        "{}({})",
        desc.sig.name,
        desc.arg_indices
            .iter()
            .map(|&index| format!("{} (#{})", metadata.column(index).name(), index))
            .join(", ")
    )
}

fn aggregate_expand_to_format_tree(
    plan: &AggregateExpand,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let sets = plan
        .grouping_sets
        .iter()
        .map(|set| {
            set.iter()
                .map(|&index| format!("{} (#{})", metadata.column(index).name(), index))
                .join(", ")
        })
        .map(|s| format!("({})", s))
        .join(", ");

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("grouping sets: [{sets}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "AggregateExpand".to_string(),
        children,
    ))
}

fn aggregate_partial_to_format_tree(
    plan: &AggregatePartial,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let group_by = plan
        .group_by
        .iter()
        .map(|&index| format!("{} (#{})", metadata.column(index).name(), index))
        .join(", ");
    let agg_funcs = plan
        .agg_funcs
        .iter()
        .map(|agg| pretty_display_agg_desc(agg, metadata))
        .join(", ");

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("group by: [{group_by}]")),
        FormatTreeNode::new(format!("aggregate functions: [{agg_funcs}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "AggregatePartial".to_string(),
        children,
    ))
}

fn aggregate_final_to_format_tree(
    plan: &AggregateFinal,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let group_by = plan
        .group_by
        .iter()
        .map(|&index| format!("{} (#{})", metadata.column(index).name(), index))
        .join(", ");

    let agg_funcs = plan
        .agg_funcs
        .iter()
        .map(|agg| pretty_display_agg_desc(agg, metadata))
        .join(", ");

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("group by: [{group_by}]")),
        FormatTreeNode::new(format!("aggregate functions: [{agg_funcs}]")),
    ];

    if let Some(limit) = &plan.limit {
        let items = FormatTreeNode::new(format!("limit: {limit}"));
        children.push(items);
    }

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "AggregateFinal".to_string(),
        children,
    ))
}

fn window_to_format_tree(
    plan: &Window,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let partition_by = plan
        .partition_by
        .iter()
        .map(|&index| format!("{} (#{})", metadata.column(index).name(), index))
        .join(", ");

    let order_by = plan
        .order_by
        .iter()
        .map(|v| format!("{} (#{})", metadata.column(v.order_by).name(), v.order_by))
        .join(", ");

    let frame = plan.window_frame.to_string();

    let func = match &plan.func {
        WindowFunction::Aggregate(agg) => pretty_display_agg_desc(agg, metadata),
        func => format!("{}", func),
    };

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("aggregate function: [{func}]")),
        FormatTreeNode::new(format!("partition by: [{partition_by}]")),
        FormatTreeNode::new(format!("order by: [{order_by}]")),
        FormatTreeNode::new(format!("frame: [{frame}]")),
    ];

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "Window".to_string(), // todo(ariesdevil): show full window expression.
        children,
    ))
}

fn sort_to_format_tree(
    plan: &Sort,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let sort_keys = plan
        .order_by
        .iter()
        .map(|sort_key| {
            let index = sort_key.order_by;
            Ok(format!(
                "{} (#{}) {} {}",
                metadata.column(index).name(),
                index,
                if sort_key.asc { "ASC" } else { "DESC" },
                if sort_key.nulls_first {
                    "NULLS FIRST"
                } else {
                    "NULLS LAST"
                }
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join(", ");

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("sort keys: [{sort_keys}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children("Sort".to_string(), children))
}

fn limit_to_format_tree(
    plan: &Limit,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!(
            "limit: {}",
            plan.limit
                .map_or("NONE".to_string(), |limit| limit.to_string())
        )),
        FormatTreeNode::new(format!("offset: {}", plan.offset)),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children("Limit".to_string(), children))
}

fn row_fetch_to_format_tree(
    plan: &RowFetch,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let table_schema = plan.source.source_info.schema();
    let projected_schema = plan.cols_to_fetch.project_schema(&table_schema);
    let fields_to_fetch = projected_schema.fields();

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!(
            "columns to fetch: [{}]",
            fields_to_fetch.iter().map(|f| f.name()).join(", ")
        )),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "RowFetch".to_string(),
        children,
    ))
}

fn range_join_to_format_tree(
    plan: &RangeJoin,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let range_join_conditions = plan
        .conditions
        .iter()
        .map(|condition| {
            let left = condition
                .left_expr
                .as_expr(&BUILTIN_FUNCTIONS)
                .sql_display();
            let right = condition
                .right_expr
                .as_expr(&BUILTIN_FUNCTIONS)
                .sql_display();
            format!("{left} {:?} {right}", condition.operator)
        })
        .collect::<Vec<_>>()
        .join(", ");
    let other_conditions = plan
        .other_conditions
        .iter()
        .map(|filter| filter.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        .collect::<Vec<_>>()
        .join(", ");

    let mut left_child = to_format_tree(&plan.left, metadata, prof_span_set)?;
    let mut right_child = to_format_tree(&plan.right, metadata, prof_span_set)?;

    left_child.payload = format!("{}(Left)", left_child.payload);
    right_child.payload = format!("{}(Right)", right_child.payload);

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("join type: {}", plan.join_type)),
        FormatTreeNode::new(format!("range join conditions: [{range_join_conditions}]")),
        FormatTreeNode::new(format!("other conditions: [{other_conditions}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(left_child);
    children.push(right_child);

    Ok(FormatTreeNode::with_children(
        match plan.range_join_type {
            RangeJoinType::IEJoin => "IEJoin".to_string(),
            RangeJoinType::Merge => "MergeJoin".to_string(),
        },
        children,
    ))
}

fn hash_join_to_format_tree(
    plan: &HashJoin,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let build_keys = plan
        .build_keys
        .iter()
        .map(|scalar| scalar.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        .join(", ");
    let probe_keys = plan
        .probe_keys
        .iter()
        .map(|scalar| scalar.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        .join(", ");
    let filters = plan
        .non_equi_conditions
        .iter()
        .map(|filter| filter.as_expr(&BUILTIN_FUNCTIONS).sql_display())
        .join(", ");

    let mut build_child = to_format_tree(&plan.build, metadata, prof_span_set)?;
    let mut probe_child = to_format_tree(&plan.probe, metadata, prof_span_set)?;

    build_child.payload = format!("{}(Build)", build_child.payload);
    probe_child.payload = format!("{}(Probe)", probe_child.payload);

    let mut children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("join type: {}", plan.join_type)),
        FormatTreeNode::new(format!("build keys: [{build_keys}]")),
        FormatTreeNode::new(format!("probe keys: [{probe_keys}]")),
        FormatTreeNode::new(format!("filters: [{filters}]")),
    ];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.push(build_child);
    children.push(probe_child);

    Ok(FormatTreeNode::with_children(
        "HashJoin".to_string(),
        children,
    ))
}

fn exchange_to_format_tree(
    plan: &Exchange,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    Ok(FormatTreeNode::with_children("Exchange".to_string(), vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        FormatTreeNode::new(format!("exchange type: {}", match plan.kind {
            FragmentKind::Init => "Init-Partition".to_string(),
            FragmentKind::Normal => format!(
                "Hash({})",
                plan.keys
                    .iter()
                    .map(|key| { key.as_expr(&BUILTIN_FUNCTIONS).sql_display() })
                    .join(", ")
            ),
            FragmentKind::Expansive => "Broadcast".to_string(),
            FragmentKind::Merge => "Merge".to_string(),
        })),
        to_format_tree(&plan.input, metadata, prof_span_set)?,
    ]))
}

fn union_all_to_format_tree(
    plan: &UnionAll,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    ))];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.extend(vec![
        to_format_tree(&plan.left, metadata, prof_span_set)?,
        to_format_tree(&plan.right, metadata, prof_span_set)?,
    ]);

    Ok(FormatTreeNode::with_children(
        "UnionAll".to_string(),
        children,
    ))
}

fn part_stats_info_to_format_tree(info: &PartStatistics) -> Vec<FormatTreeNode<String>> {
    let mut items = vec![
        FormatTreeNode::new(format!("read rows: {}", info.read_rows)),
        FormatTreeNode::new(format!("read bytes: {}", info.read_bytes)),
        FormatTreeNode::new(format!("partitions total: {}", info.partitions_total)),
        FormatTreeNode::new(format!("partitions scanned: {}", info.partitions_scanned)),
    ];

    if info.pruning_stats.segments_range_pruning_before > 0 {
        items.push(FormatTreeNode::new(format!(
            "pruning stats: [segments: <range pruning: {} to {}>, blocks: <range pruning: {} to {}, bloom pruning: {} to {}>]",
            info.pruning_stats.segments_range_pruning_before,
            info.pruning_stats.segments_range_pruning_after,
            info.pruning_stats.blocks_range_pruning_before,
            info.pruning_stats.blocks_range_pruning_after,
            info.pruning_stats.blocks_bloom_pruning_before,
            info.pruning_stats.blocks_bloom_pruning_after,
        )))
    }

    items
}

fn plan_stats_info_to_format_tree(info: &PlanStatsInfo) -> Vec<FormatTreeNode<String>> {
    vec![FormatTreeNode::new(format!(
        "estimated rows: {0:.2}",
        info.estimated_rows
    ))]
}

fn exchange_source_to_format_tree(
    plan: &ExchangeSource,
    metadata: &Metadata,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    ))];

    children.push(FormatTreeNode::new(format!(
        "source fragment: [{}]",
        plan.source_fragment_id
    )));

    Ok(FormatTreeNode::with_children(
        "ExchangeSource".to_string(),
        children,
    ))
}

fn exchange_sink_to_format_tree(
    plan: &ExchangeSink,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    ))];

    children.push(FormatTreeNode::new(format!(
        "destination fragment: [{}]",
        plan.destination_fragment_id
    )));

    children.push(to_format_tree(&plan.input, metadata, prof_span_set)?);

    Ok(FormatTreeNode::with_children(
        "ExchangeSink".to_string(),
        children,
    ))
}

fn distributed_insert_to_format_tree(
    plan: &DistributedInsertSelect,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let children = vec![to_format_tree(&plan.input, metadata, prof_span_set)?];

    Ok(FormatTreeNode::with_children(
        "DistributedInsertSelect".to_string(),
        children,
    ))
}

fn delete_partial_to_format_tree(
    _plan: &DeletePartial,
    _metadata: &Metadata,
    _prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    Ok(FormatTreeNode::new("DeletePartial".to_string()))
}

fn delete_final_to_format_tree(
    plan: &MutationAggregate,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let children = vec![to_format_tree(&plan.input, metadata, prof_span_set)?];
    Ok(FormatTreeNode::with_children(
        "DeleteFinal".to_string(),
        children,
    ))
}

fn project_set_to_format_tree(
    plan: &ProjectSet,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    ))];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.extend(vec![FormatTreeNode::new(format!(
        "set returning functions: {}",
        plan.srf_exprs
            .iter()
            .map(|(expr, _)| expr.clone().as_expr(&BUILTIN_FUNCTIONS).sql_display())
            .join(", ")
    ))]);

    children.extend(vec![to_format_tree(&plan.input, metadata, prof_span_set)?]);

    Ok(FormatTreeNode::with_children(
        "ProjectSet".to_string(),
        children,
    ))
}

fn lambda_to_format_tree(
    plan: &Lambda,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let mut children = vec![FormatTreeNode::new(format!(
        "output columns: [{}]",
        format_output_columns(plan.output_schema()?, metadata, true)
    ))];

    if let Some(info) = &plan.stat_info {
        let items = plan_stats_info_to_format_tree(info);
        children.extend(items);
    }

    append_profile_info(&mut children, prof_span_set, plan.plan_id);

    children.extend(vec![FormatTreeNode::new(format!(
        "lambda functions: {}",
        plan.lambda_funcs
            .iter()
            .map(|func| {
                let arg_exprs = func.arg_exprs.join(", ");
                let params = func.params.join(", ");
                let lambda_expr = func.lambda_expr.as_expr(&BUILTIN_FUNCTIONS).sql_display();
                format!(
                    "{}({}, {} -> {})",
                    func.func_name, arg_exprs, params, lambda_expr
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    ))]);

    children.extend(vec![to_format_tree(&plan.input, metadata, prof_span_set)?]);

    Ok(FormatTreeNode::with_children(
        "Lambda".to_string(),
        children,
    ))
}

fn runtime_filter_source_to_format_tree(
    plan: &RuntimeFilterSource,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        to_format_tree(&plan.left_side, metadata, prof_span_set)?,
        to_format_tree(&plan.right_side, metadata, prof_span_set)?,
    ];
    Ok(FormatTreeNode::with_children(
        "RuntimeFilterSource".to_string(),
        children,
    ))
}

fn materialized_cte_to_format_tree(
    plan: &MaterializedCte,
    metadata: &Metadata,
    prof_span_set: &SharedProcessorProfiles,
) -> Result<FormatTreeNode<String>> {
    let children = vec![
        FormatTreeNode::new(format!(
            "output columns: [{}]",
            format_output_columns(plan.output_schema()?, metadata, true)
        )),
        to_format_tree(&plan.left, metadata, prof_span_set)?,
        to_format_tree(&plan.right, metadata, prof_span_set)?,
    ];
    Ok(FormatTreeNode::with_children(
        "MaterializedCTE".to_string(),
        children,
    ))
}

fn format_output_columns(
    output_schema: DataSchemaRef,
    metadata: &Metadata,
    format_table: bool,
) -> String {
    output_schema
        .fields()
        .iter()
        .map(|field| match field.name().parse::<usize>() {
            Ok(column_index) => {
                let column_entry = metadata.column(column_index);
                match column_entry.table_index() {
                    Some(table_index) if format_table => match metadata
                        .table(table_index)
                        .alias_name()
                    {
                        Some(alias_name) => {
                            format!("{}.{} (#{})", alias_name, column_entry.name(), column_index)
                        }
                        None => format!(
                            "{}.{} (#{})",
                            metadata.table(table_index).name(),
                            column_entry.name(),
                            column_index,
                        ),
                    },
                    _ => format!("{} (#{})", column_entry.name(), column_index),
                }
            }
            _ => format!("#{}", field.name()),
        })
        .join(", ")
}
