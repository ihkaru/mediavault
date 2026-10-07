use crate::{
    config::Config,
    db::DbPool,
    search::MeiliClient,
    services::MediaService,
    storage::DynStorage,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub db: DbPool,
    pub storage: DynStorage,
    pub search: Arc<MeiliClient>,
    pub media: Arc<MediaService>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: DbPool,
        storage: DynStorage,
        search: MeiliClient,
    ) -> Self {
        let config = Arc::new(config);
        let search = Arc::new(search);
        let media_service = Arc::new(MediaService::new(
            config.clone(),
            db.clone(),
            storage.clone(),
            search.clone(),
        ));

        Self {
            config,
            db,
            storage,
            search,
            media: media_service,
        }
    }
}
