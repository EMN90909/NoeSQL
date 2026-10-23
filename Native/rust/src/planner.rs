use crate::reference::{Expr, Select, Table};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinPlan {
    None,
    Nested,
    Hash { left_key: String, right_key: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationalPlan {
    pub join: JoinPlan,
    pub left_rows: usize,
    pub right_rows: usize,
    pub push_where: bool,
}

fn scalar(expr: &Expr) -> bool {
    matches!(expr, Expr::Column(_) | Expr::Literal(_) | Expr::Param(_))
}

pub fn safe_predicate(expr: &Expr) -> bool {
    match expr {
        Expr::Binary(op, left, right) if op == "&&" || op == "||" => {
            safe_predicate(left) && safe_predicate(right)
        }
        Expr::Binary(op, left, right)
            if matches!(op.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=") =>
        {
            scalar(left) && scalar(right)
        }
        _ => false,
    }
}

pub fn base_only(expr: &Expr, table: &str) -> bool {
    match expr {
        Expr::Column(name) => !name.contains('.') || name.starts_with(&format!("{table}.")),
        Expr::Binary(_, left, right) => base_only(left, table) && base_only(right, table),
        Expr::Unary(_, arg) => base_only(arg, table),
        Expr::Literal(_) | Expr::Param(_) => true,
    }
}

fn equality_terms<'a>(expr: &'a Expr, out: &mut Vec<(&'a str, &'a str)>) {
    match expr {
        Expr::Binary(op, left, right) if op == "&&" => {
            equality_terms(left, out);
            equality_terms(right, out);
        }
        Expr::Binary(op, left, right) if op == "==" => {
            if let (Expr::Column(left), Expr::Column(right)) = (&**left, &**right) {
                out.push((left, right));
            }
        }
        _ => {}
    }
}

fn left_name<'a>(name: &'a str, table: &str) -> Option<&'a str> {
    let prefix = format!("{table}.");
    if name.starts_with(&prefix) {
        Some(&name[prefix.len()..])
    } else if !name.contains('.') {
        Some(name)
    } else {
        None
    }
}

fn right_name<'a>(name: &'a str, table: &str) -> Option<&'a str> {
    let prefix = format!("{table}.");
    name.strip_prefix(&prefix)
}

pub fn choose_relational_plan(select: &Select, left: &Table, right: Option<&Table>) -> RelationalPlan {
    let left_rows = left.rows.len();
    let right_rows = right.map(|table| table.rows.len()).unwrap_or(0);

    let Some(right_table) = right else {
        return RelationalPlan { join: JoinPlan::None, left_rows, right_rows, push_where: false };
    };
    let Some(join) = &select.join else {
        return RelationalPlan { join: JoinPlan::None, left_rows, right_rows, push_where: false };
    };

    let push_where = select.filter.as_ref()
        .map(|filter| safe_predicate(filter) && base_only(filter, &select.table))
        .unwrap_or(false);

    let mut pair = None;
    if safe_predicate(&join.on) {
        let mut terms = Vec::new();
        equality_terms(&join.on, &mut terms);
        for (a, b) in terms {
            if let (Some(left_key), Some(right_key)) =
                (left_name(a, &select.table), right_name(b, &join.table))
            {
                pair = Some((left_key.to_string(), right_key.to_string()));
                break;
            }
            if let (Some(left_key), Some(right_key)) =
                (left_name(b, &select.table), right_name(a, &join.table))
            {
                pair = Some((left_key.to_string(), right_key.to_string()));
                break;
            }
        }
    }

    let join = match pair {
        Some((left_key, right_key)) if left_rows.saturating_mul(right_rows) >= 64 => {
            if right_table.fields.iter().any(|field| field.name == right_key) {
                JoinPlan::Hash { left_key, right_key }
            } else {
                JoinPlan::Nested
            }
        }
        _ => JoinPlan::Nested,
    };

    RelationalPlan { join, left_rows, right_rows, push_where }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::{parse, Database, Params, Statement};

    #[test]
    fn large_equality_join_selects_hash_plan() {
        let mut db = Database::default();
        db.execute(
            r#"table lefts {
 id:int key auto
 value:int required
}
table rights {
 id:int key auto
 left_id:int required
}"#,
            &Params::new(),
        ).unwrap();

        for value in 0..8 {
            db.execute(&format!("lefts add {{value:{value}}}"), &Params::new()).unwrap();
            db.execute(&format!("rights add {{left_id:{}}}", value + 1), &Params::new()).unwrap();
        }

        let statements = parse("lefts get lefts.id,rights.id join rights on lefts.id == rights.left_id").unwrap();
        let Statement::Get(select) = &statements[0] else { panic!("expected get"); };
        let plan = choose_relational_plan(&select, &db.tables["lefts"], Some(&db.tables["rights"]));
        assert!(matches!(plan.join, JoinPlan::Hash { .. }));
    }
}
