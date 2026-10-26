use noesql_runtime::codec;
use noesql_runtime::reference::{Database, Params};
use noesql_runtime::value::Value;

#[test]
fn extended_column_values_validate_roundtrip_and_reject_invalid_mutations() {
    let mut db = Database::default();
    db.execute(
        r#"table payloads {
 id:int key auto
 event_date:date required
 created_at:datetime required
 key:uuid required unique
 payload:json required
 attachment:binary required
}
payloads add {
 event_date:"2024-02-29",
 created_at:"2026-10-26T11:25:45.001Z",
 key:"550e8400-e29b-41d4-a716-446655440000",
 payload:"{\"name\":\"Nairobi\",\"count\":2}",
 attachment:"0xdeadbeef"
}"#,
        &Params::new(),
    ).unwrap();

    let payload = codec::encode(&db).unwrap();
    let mut reopened = codec::decode(&payload).unwrap();
    let rows = reopened.execute("payloads get id,event_date,created_at,key,payload,attachment", &Params::new()).unwrap().rows;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("event_date"), Some(&Value::Text("2024-02-29".into())));
    assert_eq!(rows[0].get("attachment"), Some(&Value::Text("0xdeadbeef".into())));

    let invalid = [
        r#"payloads add {event_date:"1900-02-29",created_at:"2026-10-26T11:25:45.001Z",key:"550e8400-e29b-41d4-a716-446655440001",payload:"{}",attachment:"0x11"}"#,
        r#"payloads add {event_date:"2024-02-29",created_at:"2026-10-26T99:25:45.001Z",key:"550e8400-e29b-41d4-a716-446655440001",payload:"{}",attachment:"0x11"}"#,
        r#"payloads add {event_date:"2024-02-29",created_at:"2026-10-26T11:25:45.001Z",key:"invalid",payload:"{}",attachment:"0x11"}"#,
        r#"payloads add {event_date:"2024-02-29",created_at:"2026-10-26T11:25:45.001Z",key:"550e8400-e29b-41d4-a716-446655440001",payload:"[1,]",attachment:"0x11"}"#,
        r#"payloads add {event_date:"2024-02-29",created_at:"2026-10-26T11:25:45.001Z",key:"550e8400-e29b-41d4-a716-446655440001",payload:"{}",attachment:"0x0"}"#,
    ];
    for query in invalid {
        assert!(reopened.execute(query, &Params::new()).is_err(), "accepted malformed typed value: {query}");
    }
    assert_eq!(reopened.tables["payloads"].rows.len(), 1);
}
