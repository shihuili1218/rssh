//! `rssh group <list|add|edit|rm>` —— connection group management.

use rssh_lib::error::{AppError, AppResult};
use rssh_lib::models::Group;

use crate::ctx::CliCtx;
use crate::helpers::{confirm, die, prompt, prompt_default};

fn parse_sort_order(value: &str) -> AppResult<i32> {
    value.parse().map_err(|_| {
        AppError::config(
            "numeric_arg_invalid",
            serde_json::json!({ "field": "sort_order" }),
        )
    })
}

pub fn cmd_list_groups(conn: &CliCtx) -> AppResult<()> {
    let groups = rssh_lib::db::group::list(conn)?;
    if groups.is_empty() {
        println!("No groups.");
        return Ok(());
    }

    println!("{:<24} {:<9} ORDER", "NAME", "COLOR");
    println!("{}", "-".repeat(42));
    for group in groups {
        println!("{:<24} {:<9} {}", group.name, group.color, group.sort_order);
    }
    Ok(())
}

pub fn cmd_add_group(conn: &CliCtx) -> AppResult<()> {
    let name = prompt("Name: ");
    let color = prompt_default("Color (#RRGGBB)", "#4A6CF7");
    let sort_order = prompt_default("Sort order", "0");
    let group = Group {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        color,
        sort_order: parse_sort_order(&sort_order)?,
    };
    rssh_lib::db::group::insert(conn, &group)?;
    println!("Group '{}' created.", group.name);
    Ok(())
}

pub fn cmd_edit_group(conn: &CliCtx, name: &str) -> AppResult<()> {
    let groups = rssh_lib::db::group::list(conn)?;
    let current = groups
        .iter()
        .find(|group| group.name.eq_ignore_ascii_case(name))
        .unwrap_or_else(|| die(format!("Group '{name}' not found")));
    let mut updated = current.clone();
    updated.name = prompt_default("Name", &current.name);
    updated.color = prompt_default("Color (#RRGGBB)", &current.color);
    let sort_order = prompt_default("Sort order", &current.sort_order.to_string());
    updated.sort_order = parse_sort_order(&sort_order)?;

    rssh_lib::db::group::update(conn, &updated)?;
    println!("Group '{}' updated.", updated.name);
    Ok(())
}

pub fn cmd_rm_group(conn: &CliCtx, name: &str) -> AppResult<()> {
    let groups = rssh_lib::db::group::list(conn)?;
    let id = groups
        .iter()
        .find(|group| group.name.eq_ignore_ascii_case(name))
        .map(|group| group.id.clone())
        .unwrap_or_else(|| die(format!("Group '{name}' not found")));
    if !confirm(&format!("Delete group '{name}'?"), false) {
        return Ok(());
    }

    rssh_lib::db::group::delete(conn, &id)?;
    println!("Deleted.");
    Ok(())
}
