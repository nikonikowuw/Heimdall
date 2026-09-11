use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};

use crate::entity::personnel::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct PersonnelRepo;

impl PersonnelRepo {
    /// 分页查询人员档案列表，支持关键字模糊匹配姓名、工号或证件号
    pub async fn list_filtered<C: ConnectionTrait>(
        db: &C,
        keyword: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<(Vec<Model>, u64), DbError> {
        let mut query = Entity::find();

        if let Some(kw) = keyword {
            let kw_trim = kw.trim();
            if !kw_trim.is_empty() {
                let pattern = format!("%{kw_trim}%");
                query = query.filter(
                    Column::Name
                        .like(&pattern)
                        .or(Column::SubjectId.like(&pattern))
                        .or(Column::IdCard.like(&pattern)),
                );
            }
        }

        let total = query.clone().count(db).await.map_err(DbError::from)?;
        let items = query
            .order_by_desc(Column::CreatedAt)
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)?;

        Ok((items, total))
    }

    /// 根据 subject_id 查询人员
    pub async fn find_by_subject_id<C: ConnectionTrait>(
        db: &C,
        subject_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::SubjectId.eq(subject_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 插入新人员
    pub async fn insert<C: ConnectionTrait>(
        db: &C,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    /// 更新人员基础信息
    pub async fn update<C: ConnectionTrait>(
        db: &C,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.update(db).await.map_err(DbError::from)
    }

    /// 更新人员主头像路径
    pub async fn update_primary_photo<C: ConnectionTrait>(
        db: &C,
        subject_id: &str,
        primary_photo_path: &str,
    ) -> Result<(), DbError> {
        if let Some(model) = Self::find_by_subject_id(db, subject_id).await? {
            let mut active: ActiveModel = model.into();
            active.primary_photo_path = Set(primary_photo_path.to_string());
            active.updated_at = Set(chrono::Utc::now());
            active.update(db).await.map_err(DbError::from)?;
        }
        Ok(())
    }

    /// 删除指定 subject_id 的人员（级联触发 SQLite 外键删除人脸样本）
    pub async fn delete_by_subject_id<C: ConnectionTrait>(
        db: &C,
        subject_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::SubjectId.eq(subject_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    /// 统计人员在册总数
    pub async fn count_all<C: ConnectionTrait>(db: &C) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }
}
