// hotword_correction/repository.rs
//
// 热词表的数据库访问层。提供 list / add / delete 三个原子操作。

use sqlx::SqlitePool;

use super::Hotword;

/// 热词仓库。封装 hotwords 表的 CRUD 操作。
pub struct HotwordRepository {
    pool: SqlitePool,
}

impl HotwordRepository {
    /// 创建仓库实例。pool 由调用方克隆自 AppState.db_manager.pool()。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 查询热词。
    /// - `scope = None`：查询全部热词
    /// - `scope = Some(meeting_id)`：查询该会议专属热词 + 全局热词
    pub async fn list(&self, scope: Option<&str>) -> Result<Vec<Hotword>, String> {
        let rows = match scope {
            None => {
                sqlx::query_as::<_, Hotword>(
                    "SELECT id, word, category, scope, created_at FROM hotwords ORDER BY created_at DESC",
                )
                .fetch_all(&self.pool)
                .await
            }
            Some(meeting_id) => {
                sqlx::query_as::<_, Hotword>(
                    "SELECT id, word, category, scope, created_at FROM hotwords \
                     WHERE scope = 'global' OR scope = ? ORDER BY created_at DESC",
                )
                .bind(meeting_id)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|e| format!("Failed to list hotwords: {}", e))?;

        Ok(rows)
    }

    /// 新增热词。返回新生成的热词 ID。
    pub async fn add(
        &self,
        word: &str,
        category: Option<&str>,
        scope: &str,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO hotwords (id, word, category, scope) VALUES (?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(word)
        .bind(category)
        .bind(scope)
        .execute(&self.pool)
        .await
        .map_err(|e| format!("Failed to insert hotword: {}", e))?;

        Ok(id)
    }

    /// 按 ID 删除热词。
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM hotwords WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| format!("Failed to delete hotword: {}", e))?;
        Ok(())
    }
}
