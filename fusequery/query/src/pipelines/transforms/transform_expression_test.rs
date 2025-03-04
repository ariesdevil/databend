// Copyright 2020-2021 The Datafuse Authors.
//
// SPDX-License-Identifier: Apache-2.0.

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_transform_expression() -> anyhow::Result<()> {
    use std::sync::Arc;

    use common_planners::*;
    use futures::TryStreamExt;
    use pretty_assertions::assert_eq;

    use crate::pipelines::processors::*;
    use crate::pipelines::transforms::*;

    let ctx = crate::tests::try_create_context()?;
    let test_source = crate::tests::NumberTestData::create(ctx.clone());

    let mut pipeline = Pipeline::create(ctx.clone());
    let source = test_source.number_source_transform_for_test(8)?;
    pipeline.add_source(Arc::new(source))?;

    if let PlanNode::Expression(plan) = PlanBuilder::create(test_source.number_schema_for_test()?)
        .expression(
            &[col("number"), col("number"), add(col("number"), lit(1u8))],
            ""
        )?
        .build()?
    {
        pipeline.add_simple_transform(|| {
            Ok(Box::new(ExpressionTransform::try_create(
                plan.input.schema(),
                plan.schema.clone(),
                plan.exprs.clone()
            )?))
        })?;
    }

    let stream = pipeline.execute().await?;
    let result = stream.try_collect::<Vec<_>>().await?;
    let block = &result[0];
    assert_eq!(block.num_columns(), 2);

    let expected = vec![
        "+--------+--------------+",
        "| number | (number + 1) |",
        "+--------+--------------+",
        "| 0      | 1            |",
        "| 1      | 2            |",
        "| 2      | 3            |",
        "| 3      | 4            |",
        "| 4      | 5            |",
        "| 5      | 6            |",
        "| 6      | 7            |",
        "| 7      | 8            |",
        "+--------+--------------+",
    ];
    common_datablocks::assert_blocks_sorted_eq(expected, result.as_slice());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_transform_expression_error() -> anyhow::Result<()> {
    use std::sync::Arc;

    use common_planners::*;
    use pretty_assertions::assert_eq;

    use crate::pipelines::processors::*;

    let ctx = crate::tests::try_create_context()?;
    let test_source = crate::tests::NumberTestData::create(ctx.clone());

    let mut pipeline = Pipeline::create(ctx.clone());
    let source = test_source.number_source_transform_for_test(8)?;
    pipeline.add_source(Arc::new(source))?;

    let result = PlanBuilder::create(test_source.number_schema_for_test()?).project(&[
        col("xnumber"),
        col("number"),
        add(col("number"), lit(1u8))
    ]);
    let actual = format!("{}", result.err().unwrap());
    let expect = "Code: 1002, displayText = Invalid argument error: Unable to get field named \"xnumber\". Valid fields: [\"number\"].";
    assert_eq!(expect, actual);

    Ok(())
}
