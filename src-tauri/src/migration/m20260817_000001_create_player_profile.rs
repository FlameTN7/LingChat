use sea_orm_migration::prelude::*;

/// 玩家档案 DB 表迁移（已停用，仅保留占位）。
///
/// 玩家身份已改为文件驱动（`game_data/player/settings.yml`），不再使用
/// `player_profile` 表。但老库的 `seaql_migrations` 里已经记录过这个版本，
/// 直接删除迁移文件会让升级到本版本时启动报错；因此保留占位迁移：
/// - `up()` 改为 no-op：老库版本号仍然被记录，不会重新建表；
///   新库也只记录版本，不再创建表。
/// - `down()` 保留 drop 并加 `if_exists()`：即使表不存在也不会报错。
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // 老库的 seaql_migrations 已记录本版本：这里只占位记录，不建表；
        // 新库同理只推进版本号。文件驱动的玩家档案与 DB 表彻底解耦。
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 兼容老库残留表：回滚时若表还在就删掉，不存在则静默通过。
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("player_profile"))
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}
