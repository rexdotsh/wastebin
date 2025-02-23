use crate::cache::Key;
use crate::crypto::Password;
use crate::db::read::Entry;
use crate::handlers::extract::Theme;
use crate::handlers::html::{ErrorResponse, PasswordInput, make_error};
use crate::highlight::Html;
use crate::{Cache, Database, Error, Highlighter, Page};
use askama::Template;
use axum::extract::{Form, Path, State};
use axum::response::{IntoResponse, Response};
use axum_extra::extract::SignedCookieJar;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct PasswordForm {
    password: String,
}

/// Paste view showing the formatted paste.
#[derive(Template)]
#[template(path = "formatted.html")]
pub struct Paste {
    page: Page,
    key: Key,
    theme: Option<Theme>,
    can_delete: bool,
    /// If the paste still in the database and can be fetched with another request.
    is_available: bool,
    html: String,
    title: String,
}

#[expect(clippy::too_many_arguments)]
pub async fn get(
    State(cache): State<Cache>,
    State(page): State<Page>,
    State(db): State<Database>,
    State(highlighter): State<Highlighter>,
    Path(id): Path<String>,
    theme: Option<Theme>,
    jar: SignedCookieJar,
    form: Option<Form<PasswordForm>>,
) -> Result<Response, ErrorResponse> {
    async {
        let password = form.map(|form| Password::from(form.password.as_bytes().to_vec()));
        let key: Key = id.parse()?;

        let (data, is_available) = match db.get(key.id, password.clone()).await {
            Ok(Entry::Regular(data)) => (data, true),
            Ok(Entry::Burned(data)) => (data, false),
            Ok(Entry::Expired) => return Err(Error::NotFound),
            Err(Error::NoPassword) => {
                return Ok(PasswordInput {
                    page: page.clone(),
                    theme: theme.clone(),
                    id,
                }
                .into_response());
            }
            Err(err) => return Err(err),
        };

        let can_be_deleted = jar
            .get("uid")
            .map(|cookie| cookie.value().parse::<i64>())
            .transpose()
            .map_err(|err| Error::CookieParsing(err.to_string()))?
            .zip(data.uid)
            .is_some_and(|(user_uid, owner_uid)| user_uid == owner_uid);

        let title = data.title.clone().unwrap_or_default();

        let html = if let Some(html) = cache.get(&key) {
            tracing::trace!(?key, "found cached item");

            html
        } else {
            let html = highlighter.highlight(data, key.ext.clone()).await?;

            if is_available && password.is_none() {
                tracing::trace!(?key, "cache item");
                cache.put(key.clone(), html.clone());
            }

            html
        };

        Ok(Paste::new(
            key,
            html,
            theme.clone(),
            can_be_deleted,
            is_available,
            title,
            page.clone(),
        )
        .into_response())
    }
    .await
    .map_err(|err| make_error(err, page, theme))
}

impl Paste {
    /// Construct new paste view from cache `key` and paste `html`.
    pub fn new(
        key: Key,
        html: Html,
        theme: Option<Theme>,
        can_delete: bool,
        is_available: bool,
        title: String,
        page: Page,
    ) -> Self {
        let html = html.into_inner();

        Self {
            page,
            key,
            theme,
            can_delete,
            is_available,
            html,
            title,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::Client;
    use reqwest::StatusCode;

    #[tokio::test]
    async fn unknown_paste() -> Result<(), Box<dyn std::error::Error>> {
        let client = Client::new().await;

        let res = client.get("/000000").send().await?;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        Ok(())
    }
}
