use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "running_script")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub script_folder: String,
    #[sea_orm(column_type = "Text")]
    pub variable_info: String,
    pub current_chapter: String,
    pub event_sequence: i32,
    /// 玩家阅读位置——章节 key（前端上报，读档优先据此恢复；NULL = 无上报记录）
    pub player_read_chapter: Option<String>,
    /// 玩家阅读位置——事件序号（前端上报，读档优先据此恢复；NULL = 无上报记录）
    pub player_read_sequence: Option<i32>,
    pub save_id: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::save::Entity",
        from = "Column::SaveId",
        to = "super::save::Column::Id"
    )]
    Save,
}

impl Related<super::save::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Save.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
