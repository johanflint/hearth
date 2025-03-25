use serde::de::DeserializeOwned;

#[derive(Debug, PartialEq)]
pub struct ServerSentEvent<T> {
    pub id: Option<String>,
    pub event: Option<String>,
    pub retry: Option<usize>,
    pub comment: Option<String>,
    pub data: Option<T>,
}

impl<T> ServerSentEvent<T>
where
    T: DeserializeOwned,
{
    pub fn from_str(s: &str) -> Result<ServerSentEvent<T>, serde_json::Error> {
        let mut id = None;
        let mut event = None;
        let mut retry = None;
        let mut comment = None;
        let mut data = None;

        for line in s.lines() {
            if line.starts_with("id:") {
                id = Some(line["id:".len()..].trim().to_string());
            } else if line.starts_with("event:") {
                event = Some(line["event:".len()..].trim().to_string());
            } else if line.starts_with("retry:") {
                retry = line["retry:".len()..].trim().parse::<usize>().ok();
            } else if line.starts_with(":") {
                comment = Some(line[":".len()..].trim().to_string());
            } else if line.starts_with("data:") {
                let data_str = line["data:".len()..].trim().to_string();
                data = Some(serde_json::from_str(&data_str)?);
            }
        }

        Ok(ServerSentEvent {
            id,
            event,
            retry,
            comment,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde::Deserialize;

    #[derive(Deserialize, PartialEq, Debug)]
    struct Data {
        name: String,
    }

    #[rstest]
    #[case("id: 42", ServerSentEvent { id: Some("42".to_string()), event: None, retry: None, comment: None, data: None, })]
    #[case("event: disconnect", ServerSentEvent { id: None, event: Some("disconnect".to_string()), retry: None, comment: None, data: None, })]
    #[case("retry: 1337", ServerSentEvent { id: None, event: None, retry: Some(1337), comment: None, data: None, })]
    #[case("retry: yes", ServerSentEvent { id: None, event: None, retry: None, comment: None, data: None, })]
    #[case(": hi", ServerSentEvent { id: None, event: None, retry: None, comment: Some("hi".to_string()), data: None, })]
    #[case(r#"data: { "name": "Johan" } "#, ServerSentEvent { id: None, event: None, retry: None, comment: None, data: Some(Data { name: "Johan".to_string(), }), })]
    fn deserializes_a_single_field(#[case] data: &str, #[case] expected: ServerSentEvent<Data>) -> Result<(), serde_json::Error> {
        let result: ServerSentEvent<Data> = ServerSentEvent::from_str(data)?;

        assert_eq!(result, expected);
        Ok(())
    }

    #[test]
    fn deserializes_all_fields() -> Result<(), serde_json::Error> {
        let data = "id: 42\nevent: disconnect\nretry: 1337\n: hi\ndata: { \"name\": \"Johan\" }";

        let result: ServerSentEvent<Data> = ServerSentEvent::from_str(data)?;

        assert_eq!(
            result,
            ServerSentEvent {
                id: Some("42".to_string()),
                event: Some("disconnect".to_string()),
                retry: Some(1337),
                comment: Some("hi".to_string()),
                data: Some(Data { name: "Johan".to_string() }),
            }
        );
        Ok(())
    }

    #[test]
    fn deserialize_fails_if_data_deserialization_fails() -> Result<(), serde_json::Error> {
        let data = "data: no json";

        let result = ServerSentEvent::<Data>::from_str(data);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "expected ident at line 1 column 2");
        Ok(())
    }
}
