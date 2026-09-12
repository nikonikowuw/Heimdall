use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Statement,
};

use crate::entity::gallery_face::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct GalleryFaceRepo;

impl GalleryFaceRepo {
    /// 查询指定人员的所有人脸特征样本，按主头像优先、创建时间升序排列
    pub async fn list_by_subject_id<C: ConnectionTrait>(
        db: &C,
        subject_id: &str,
    ) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .filter(Column::SubjectId.eq(subject_id))
            .order_by_desc(Column::IsPrimary)
            .order_by_asc(Column::CreatedAt)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 统计指定人员已录入的人脸特征样本数量
    pub async fn count_by_subject_id<C: ConnectionTrait>(
        db: &C,
        subject_id: &str,
    ) -> Result<u64, DbError> {
        Entity::find()
            .filter(Column::SubjectId.eq(subject_id))
            .count(db)
            .await
            .map_err(DbError::from)
    }

    /// 按多个 subject_id 批量统计人脸样本数量，返回 HashMap<subject_id, count>
    pub async fn count_batch_by_subject_ids<C: ConnectionTrait>(
        db: &C,
        subject_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, u64>, DbError> {
        if subject_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let rows = Entity::find()
            .column(Column::SubjectId)
            .column(Column::Id)
            .filter(Column::SubjectId.is_in(subject_ids.iter().copied()))
            .all(db)
            .await?;
        let mut map: std::collections::HashMap<String, u64> =
            std::collections::HashMap::with_capacity(subject_ids.len());
        for row in rows {
            let sid = row.subject_id;
            *map.entry(sid).or_default() += 1;
        }
        Ok(map)
    }

    /// 根据 face_id 查询单张人脸样本
    pub async fn find_by_face_id<C: ConnectionTrait>(
        db: &C,
        face_id: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::FaceId.eq(face_id))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 插入新人脸样本
    pub async fn insert<C: ConnectionTrait>(
        db: &C,
        active_model: ActiveModel,
    ) -> Result<Model, DbError> {
        active_model.insert(db).await.map_err(DbError::from)
    }

    /// 根据 face_id 删除指定样本
    pub async fn delete_by_face_id<C: ConnectionTrait>(
        db: &C,
        face_id: &str,
    ) -> Result<u64, DbError> {
        let res = Entity::delete_many()
            .filter(Column::FaceId.eq(face_id))
            .exec(db)
            .await?;
        Ok(res.rows_affected)
    }

    /// 删除指定 subject_id 的所有人脸样本
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

    /// 将指定样本设为主头像，并将该人员其他样本的主头像标记重置
    pub async fn set_primary<C: ConnectionTrait>(
        db: &C,
        subject_id: &str,
        face_id: &str,
    ) -> Result<(), DbError> {
        // 1. 将该人员的所有样本 is_primary 置 0
        db.execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE gallery_faces SET is_primary = 0 WHERE subject_id = ?",
            [subject_id.into()],
        ))
        .await
        .map_err(DbError::from)?;

        // 2. 将目标样本 is_primary 置 1
        db.execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE gallery_faces SET is_primary = 1 WHERE face_id = ? AND subject_id = ?",
            [face_id.into(), subject_id.into()],
        ))
        .await
        .map_err(DbError::from)?;

        Ok(())
    }

    /// 查询全量有效人脸特征向量，用于常驻内存特征索引构建
    pub async fn list_all_valid_vectors<C: ConnectionTrait>(db: &C) -> Result<Vec<Model>, DbError> {
        Entity::find()
            .order_by_asc(Column::Id)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    /// 统计系统总人脸特征样本数
    pub async fn count_all<C: ConnectionTrait>(db: &C) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }
}
