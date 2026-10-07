use crate::{
    config::Config,
    db::DbPool,
    search::MeiliClient,
    storage::DynStorage,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: DbPool,
    pub storage: DynStorage,
    pub search: Arc<MeiliClient>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: DbPool,
        storage: DynStorage,
        search: MeiliClient,
    ) -> Self {
        Self {
            config: Arc::new(config),
            db,
            storage,
            search: Arc::new(search),
        }
    }
}
