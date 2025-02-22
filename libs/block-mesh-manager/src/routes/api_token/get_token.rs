use crate::database::api_token::get_api_token_by_user_id_and_status::get_api_token_by_usr_and_status;
use crate::database::user::get_user_by_email::get_user_opt_by_email;
use crate::errors::error::Error;
use crate::startup::application::AppState;
use crate::utils::verify_cache::verify_with_cache;
use axum::extract::State;
use axum::{Extension, Json};
use block_mesh_common::interfaces::server_api::{
    GetTokenRequest, GetTokenResponse, GetTokenResponseEnum,
};
use block_mesh_manager_database_domain::domain::api_token::ApiTokenStatus;
use database_utils::utils::instrument_wrapper::{commit_txn, create_txn};
use sqlx::PgPool;
use std::sync::Arc;

#[tracing::instrument(name = "get_token", skip(pool, state))]
pub async fn handler(
    Extension(pool): Extension<PgPool>,
    State(state): State<Arc<AppState>>,
    Json(body): Json<GetTokenRequest>,
) -> Result<Json<GetTokenResponse>, Error> {
    let email = body.email.clone().to_ascii_lowercase();
    let key = (email.clone(), body.password.clone());
    let get_token_map = &state.get_token_map;

    if let Some(entry) = get_token_map.get(&key) {
        return match entry.value() {
            GetTokenResponseEnum::GetTokenResponse(r) => Ok(Json(r.clone())),
            GetTokenResponseEnum::UserNotFound => Err(Error::UserNotFound),
            GetTokenResponseEnum::PasswordMismatch => Err(Error::PasswordMismatch),
            GetTokenResponseEnum::ApiTokenNotFound => Err(Error::ApiTokenNotFound),
        };
    }
    let mut transaction = create_txn(&pool).await?;

    let user = match get_user_opt_by_email(&mut transaction, &email).await {
        Ok(user) => match user {
            Some(user) => user,
            None => {
                get_token_map.insert(key, GetTokenResponseEnum::UserNotFound);
                commit_txn(transaction).await?;
                return Err(Error::UserNotFound);
            }
        },
        Err(_) => {
            get_token_map.insert(key, GetTokenResponseEnum::UserNotFound);
            commit_txn(transaction).await?;
            return Err(Error::UserNotFound);
        }
    };

    if !verify_with_cache(body.password.as_ref(), user.password.as_ref()).await {
        get_token_map.insert(key, GetTokenResponseEnum::PasswordMismatch);
        commit_txn(transaction).await?;
        return Err(Error::PasswordMismatch);
    }

    let api_token =
        match get_api_token_by_usr_and_status(&mut transaction, &user.id, ApiTokenStatus::Active)
            .await
        {
            Ok(api_token) => match api_token {
                Some(api_token) => api_token,
                None => {
                    get_token_map.insert(key, GetTokenResponseEnum::ApiTokenNotFound);
                    commit_txn(transaction).await?;
                    return Err(Error::ApiTokenNotFound);
                }
            },
            Err(_) => {
                get_token_map.insert(key, GetTokenResponseEnum::ApiTokenNotFound);
                commit_txn(transaction).await?;
                return Err(Error::ApiTokenNotFound);
            }
        };

    let response = GetTokenResponse {
        api_token: Some(*api_token.token.as_ref()),
        message: None,
    };

    get_token_map.insert(
        key,
        GetTokenResponseEnum::GetTokenResponse(response.clone()),
    );
    commit_txn(transaction).await?;
    Ok(Json(response))
}
