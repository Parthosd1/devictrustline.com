use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::User;
use crate::db::{uuid_of, Assign, Param, Q};
use crate::error::{AppError, Result};
use crate::validate::{path_uuid, Input, Opt, Str, EMAIL};
use crate::web::Body;
use crate::AppState;

const SELECT: &str = r#"
  SELECT p.id, p.name, p.email, p.employee_id AS "employeeId", p.department, p.is_active AS "isActive",
         (SELECT count(*)::int FROM assets a WHERE a.assigned_person_id = p.id) AS "assetCount"
    FROM people p"#;

struct Fields {
    name: Opt<String>,
    email: Opt<String>,
    employee_id: Opt<String>,
    department: Opt<String>,
}

fn fields(input: &mut Input, partial: bool) -> Fields {
    let name = if partial { Str::req(1, 120).optional() } else { Str::req(1, 120) };
    Fields {
        name: input.str("name", name),
        // An empty email is the same as none.
        email: input.str("email", Str { empty_null: true, ..EMAIL.nullish() }),
        employee_id: input.str("employeeId", Str::text(60)),
        department: input.str("department", Str::text(120)),
    }
}

pub async fn list(State(state): State<AppState>, user: User) -> Result<Json<Value>> {
    user.require("assets:read")?;
    let mut db = state.pool.acquire().await?;
    let items = Q::new(format!("{SELECT} WHERE p.org_id = $1 ORDER BY p.is_active DESC, p.name"))
        .bind(user.org_id)
        .all(&mut db)
        .await?;
    Ok(Json(json!({ "items": items })))
}

pub async fn create(State(state): State<AppState>, user: User, Body(body): Body) -> Result<(StatusCode, Json<Value>)> {
    user.require("people:write")?;
    let mut input = Input::body(&body);
    let f = fields(&mut input, false);
    input.strict()?;
    let mut db = state.pool.acquire().await?;
    let row = Q::new(
        "INSERT INTO people (org_id, name, email, employee_id, department) VALUES ($1, $2, $3::citext, $4, $5) RETURNING id",
    )
    .bind(user.org_id)
    .bind(f.name.into_option())
    .bind(f.email.into_option())
    .bind(f.employee_id.into_option())
    .bind(f.department.into_option())
    .one(&mut db)
    .await?;
    let person = Q::new(format!("{SELECT} WHERE p.id = $1")).bind(uuid_of(&row["id"])).one(&mut db).await?;
    Ok((StatusCode::CREATED, Json(person)))
}

pub async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<String>,
    Body(body): Body,
) -> Result<Json<Value>> {
    user.require("people:write")?;
    let id = path_uuid(&id)?;
    let mut input = Input::body(&body);
    let f = fields(&mut input, true);
    let is_active = input.boolean("isActive", false);
    input.strict()?;

    let mut assign = Assign::new(3);
    for (column, cast, value) in [
        ("name", "", f.name),
        ("email", "::citext", f.email),
        ("employee_id", "", f.employee_id),
        ("department", "", f.department),
    ] {
        if value.is_present() {
            assign.set(column, cast, Param::Text(value.into_option()));
        }
    }
    if let Opt::Val(active) = is_active {
        assign.set("is_active", "", Param::Bool(Some(active)));
    }

    let mut db = state.pool.acquire().await?;
    if !assign.is_empty() {
        let sql = format!("UPDATE people SET {}, updated_at = now() WHERE id = $1 AND org_id = $2", assign.sql());
        let count = assign.bind_to(Q::new(sql).bind(id).bind(user.org_id)).run(&mut db).await?;
        if count == 0 {
            return Err(AppError::not_found("Person"));
        }
    }
    let person = Q::new(format!("{SELECT} WHERE p.id = $1 AND p.org_id = $2"))
        .bind(id)
        .bind(user.org_id)
        .opt(&mut db)
        .await?
        .ok_or_else(|| AppError::not_found("Person"))?;
    Ok(Json(person))
}
