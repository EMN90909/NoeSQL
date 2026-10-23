use noesql_runtime::codec;
use noesql_runtime::reference::{lex, Database, Params};
use noesql_runtime::value::Value;

fn params() -> Params { Params::new() }

#[test]
fn lexer_enforces_comments_strings_limits_and_reserved_identifiers() {
    assert!(lex("/* ok */ table users { id:int }").is_ok());
    assert_eq!(lex("/* never closes").unwrap_err().code, "NOE001");
    assert_eq!(lex("table users { name:text = \"bad\\q\" }").unwrap_err().code, "NOE003");

    let nested = format!("{}1{}", "(".repeat(65), ")".repeat(65));
    assert_eq!(lex(&nested).unwrap_err().code, "NOE007");

    let mut db = Database::default();
    assert_eq!(
        db.execute("table __proto__ { id:int }", &params()).unwrap_err().code,
        "NOE019"
    );
}

#[test]
fn required_unique_key_auto_and_unknown_fields_are_enforced() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 email:text required unique
 enabled:bool required = true
}"#,
        &params(),
    ).unwrap();

    assert_eq!(db.execute("users add {email:\"a@example.com\"}", &params()).unwrap().affected, 1);
    assert_eq!(db.execute("users add {email:\"b@example.com\"}", &params()).unwrap().affected, 1);

    let duplicate = db.execute("users add {email:\"a@example.com\"}", &params()).unwrap_err();
    assert_eq!(duplicate.code, "NOE212");

    let unknown = db.execute("users add {email:\"c@example.com\",ghost:1}", &params()).unwrap_err();
    assert_eq!(unknown.code, "NOE213");

    let rows = db.execute("users get id,email sort id", &params()).unwrap().rows;
    assert_eq!(rows[0].get("id"), Some(&Value::Int(1)));
    assert_eq!(rows[1].get("id"), Some(&Value::Int(2)));
}

#[test]
fn alter_add_backfills_default_and_preserves_existing_rows() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 name:text required
}
users add {name:"Amina"}"#,
        &params(),
    ).unwrap();

    db.execute("alter users add { active:bool required = true }", &params()).unwrap();
    let rows = db.execute("users get id,name,active", &params()).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("active"), Some(&Value::Bool(true)));
}

#[test]
fn atomic_scope_rolls_back_all_mutations_on_constraint_error() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 name:text required unique
}
users add {name:"Amina"}"#,
        &params(),
    ).unwrap();

    let result = db.execute(
        r#"atomic {
users add {name:"Brian"}
users add {name:"Amina"}
}"#,
        &params(),
    );
    assert!(result.is_err());
    let rows = db.execute("users get name sort name", &params()).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name"), Some(&Value::Text("Amina".into())));
}

#[test]
fn left_join_keeps_unmatched_rows() {
    let mut db = Database::default();
    db.execute(
        r#"table teams {
 id:int key auto
 name:text required
}
table tickets {
 id:int key auto
 team_id:ref teams required
 title:text required
}
teams add {name:"Build"}
teams add {name:"Ops"}
tickets add {team_id:1,title:"Compiler"}"#,
        &params(),
    ).unwrap();

    let rows = db.execute(
        "teams get teams.name,tickets.title left join tickets on teams.id == tickets.team_id sort teams.name",
        &params(),
    ).unwrap().rows;

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get("teams.name"), Some(&Value::Text("Build".into())));
    assert_eq!(rows[0].get("tickets.title"), Some(&Value::Text("Compiler".into())));
    assert_eq!(rows[1].get("teams.name"), Some(&Value::Text("Ops".into())));
    assert_eq!(rows[1].get("tickets.title"), Some(&Value::Null));
}

#[test]
fn aggregate_group_having_sort_skip_take_match_expected_rows() {
    let mut db = Database::default();
    db.execute(
        r#"table teams {
 id:int key auto
 name:text required
}
table tickets {
 id:int key auto
 team_id:ref teams required
 effort:int required
}
teams add {name:"Build"}
teams add {name:"Ops"}
tickets add {team_id:1,effort:5}
tickets add {team_id:1,effort:3}
tickets add {team_id:2,effort:2}"#,
        &params(),
    ).unwrap();

    let rows = db.execute(
        "tickets get teams.name,count(*) as tickets,sum(effort) as effort join teams on team_id == teams.id group teams.name having effort >= 5 sort teams.name take 1",
        &params(),
    ).unwrap().rows;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("teams.name"), Some(&Value::Text("Build".into())));
    assert_eq!(rows[0].get("tickets"), Some(&Value::Int(2)));
    assert_eq!(rows[0].get("effort"), Some(&Value::Int(8)));
}

#[test]
fn bound_parameters_are_typed_values_not_source_interpolation() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 age:int required
}
users add {age:17}
users add {age:21}"#,
        &params(),
    ).unwrap();

    let mut bound = Params::new();
    bound.insert("min".into(), Value::Int(18));
    let rows = db.execute("users get id,age where age >= $min", &bound).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("age"), Some(&Value::Int(21)));

    bound.insert("min".into(), Value::Text("18 || true".into()));
    assert!(db.execute("users get id where age >= $min", &bound).is_err());
}

#[test]
fn rust_runtime_handles_thousands_of_rows_without_js_or_node() {
    let mut db = Database::default();
    db.execute(
        r#"table events {
 id:int key auto
 value:int required
}"#,
        &params(),
    ).unwrap();

    for value in 0..2_000 {
        db.execute(&format!("events add {{value:{value}}}"), &params()).unwrap();
    }
    let rows = db.execute("events get id,value where value >= 1990 sort value", &params()).unwrap().rows;
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[0].get("value"), Some(&Value::Int(1990)));
}

#[test]
fn snapshot_codec_rejects_truncation_trailing_data_and_bad_version() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 name:text required
}
users add {name:"Amina"}"#,
        &params(),
    ).unwrap();

    let encoded = codec::encode(&db).unwrap();

    for cut in [0usize, 1, 7, 8, 11, encoded.len() - 1] {
        assert!(codec::decode(&encoded[..cut]).is_err());
    }

    let mut trailing = encoded.clone();
    trailing.push(0xff);
    assert!(codec::decode(&trailing).is_err());

    let mut bad_version = encoded;
    bad_version[8..12].copy_from_slice(&99u32.to_le_bytes());
    assert!(codec::decode(&bad_version).is_err());
}


#[test]
fn foreign_keys_require_existing_target_and_block_referenced_delete() {
    let mut db = Database::default();
    db.execute(
        r#"table teams {
 id:int key auto
 name:text required
}
table tickets {
 id:int key auto
 team_id:ref teams required
 title:text required
}
teams add {name:"Build"}"#,
        &params(),
    ).unwrap();

    let missing = db.execute("tickets add {team_id:99,title:\"Ghost\"}", &params()).unwrap_err();
    assert_eq!(missing.code, "NOE205");

    db.execute("tickets add {team_id:1,title:\"Compiler\"}", &params()).unwrap();
    let blocked = db.execute("teams where id == 1 delete", &params()).unwrap_err();
    assert_eq!(blocked.code, "NOE208");

    db.execute("tickets where id == 1 delete", &params()).unwrap();
    assert_eq!(db.execute("teams where id == 1 delete", &params()).unwrap().affected, 1);
}

#[test]
fn indexes_and_named_queries_replace_reference_catalog_behavior() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 name:text required
 age:int required
}
index users_name on users(name)
query adults(min_age:int = 18) {
 users get id,name,age where age >= $min_age sort id
}
users add {name:"Young",age:16}
users add {name:"Adult",age:21}"#,
        &params(),
    ).unwrap();

    assert_eq!(db.indexes["users_name"].table, "users");
    assert_eq!(db.indexes["users_name"].columns, vec!["name".to_string()]);

    let default_rows = db.run_named("adults", &Params::new()).unwrap().rows;
    assert_eq!(default_rows.len(), 1);
    assert_eq!(default_rows[0].get("name"), Some(&Value::Text("Adult".into())));

    let mut bound = Params::new();
    bound.insert("min_age".into(), Value::Int(15));
    assert_eq!(db.run_named("adults", &bound).unwrap().rows.len(), 2);

    bound.insert("unexpected".into(), Value::Int(1));
    assert_eq!(db.run_named("adults", &bound).unwrap_err().code, "NOE133");
}

#[test]
fn schema_catalog_survives_rust_codec_round_trip() {
    let mut db = Database::default();
    db.execute(
        r#"db catalog
table users {
 id:int key auto
 name:text required
}
index users_name on users(name)
query by_name(name:text) {
 users get id,name where name == $name
}
users add {name:"Amina"}"#,
        &params(),
    ).unwrap();

    let encoded = codec::encode(&db).unwrap();
    let decoded = codec::decode(&encoded).unwrap();
    assert_eq!(decoded.name.as_deref(), Some("catalog"));
    assert_eq!(decoded.version, 1);
    assert!(decoded.indexes.contains_key("users_name"));
    assert!(decoded.queries.contains_key("by_name"));

    let mut bound = Params::new();
    bound.insert("name".into(), Value::Text("Amina".into()));
    assert_eq!(decoded.run_named("by_name", &bound).unwrap().rows.len(), 1);
}


#[test]
fn whole_multi_statement_program_rolls_back_on_late_failure() {
    let mut db = Database::default();
    db.execute(
        r#"table users {
 id:int key auto
 name:text required unique
}
users add {name:"Seed"}"#,
        &params(),
    ).unwrap();

    let result = db.execute(
        r#"users add {name:"First"}
users add {name:"Seed"}"#,
        &params(),
    );
    assert!(result.is_err());

    let rows = db.execute("users get id,name sort id", &params()).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name"), Some(&Value::Text("Seed".into())));
}
