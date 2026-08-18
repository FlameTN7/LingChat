use sea_orm::Statement;
use sea_orm_migration::prelude::*;

/// 为 running_script 增加「玩家阅读位置」两列。
///
/// 背景：剧本引擎预跑，存档记录的是**引擎执行位置**（event_sequence），可能超前于
/// 玩家实际阅读到的位置；读档从引擎位置续跑会让玩家觉得"跳剧情"。前端在玩家读到
/// 每条消息时上报「玩家阅读位置」（章节 + 事件序号），存到这两列，读档优先据此恢复。
///
/// 稳定性说明：只加可空列，SQLite 元数据级操作，不动已有行/索引；旧库升级后旧行
/// 两列自动为 NULL，读档时回退到引擎位置，行为不变。
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 防御性预检：SQLite 的 ADD COLUMN 没有 IF NOT EXISTS 语法，先查
        // pragma_table_info 确认列不存在再执行，防重复执行时报「duplicate column」。
        let existing: Vec<String> = manager
            .get_connection()
            .query_all(Statement::from_string(
                manager.get_database_backend(),
                "SELECT name FROM pragma_table_info('running_script')".to_string(),
            ))
            .await?
            .into_iter()
            .map(|row| {
                row.try_get_by_index::<String>(0)
                    .unwrap_or_else(|_| String::new())
            })
            .collect();

        if !existing.iter().any(|c| c == "player_read_chapter") {
            manager
                .alter_table(
                    Table::alter()
                        .table(RunningScript::Table)
                        .add_column(
                            ColumnDef::new(RunningScript::PlayerReadChapter)
                                .string()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }
        if !existing.iter().any(|c| c == "player_read_sequence") {
            manager
                .alter_table(
                    Table::alter()
                        .table(RunningScript::Table)
                        .add_column(
                            ColumnDef::new(RunningScript::PlayerReadSequence)
                                .integer()
                                .null(),
                        )
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(RunningScript::Table)
                    .drop_column(RunningScript::PlayerReadChapter)
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(RunningScript::Table)
                    .drop_column(RunningScript::PlayerReadSequence)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum RunningScript {
    Table,
    PlayerReadChapter,
    PlayerReadSequence,
}
