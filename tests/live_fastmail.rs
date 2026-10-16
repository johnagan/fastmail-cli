//! Opt-in tests against a Fastmail test account. Nothing runs by default.
use anyhow::{Context, Result, ensure};
use fastmail_cli::{
    config::Config,
    dav::DavClient,
    jmap::{self, DataType, NoteEdit},
};
use serde_json::json;

fn write_opt_in() -> Result<()> {
    ensure!(
        std::env::var("FASTMAIL_LIVE_WRITE_TESTS").as_deref() == Ok("1"),
        "Set FASTMAIL_LIVE_WRITE_TESTS=1 with credentials for a test account"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires live Fastmail credentials"]
async fn live_session_and_mailbox_reads() -> Result<()> {
    let client = jmap::authenticated_client().await?;
    ensure!(!client.session()?.username.is_empty());
    client.fetch_mailboxes().await?;
    ensure!(client.records(DataType::Email, Some(&[])).await?["state"].is_string());
    Ok(())
}

#[tokio::test]
#[ignore = "creates/deletes temporary contacts and notes; requires FASTMAIL_LIVE_WRITE_TESTS=1"]
async fn live_contact_media_and_rich_notes() -> Result<()> {
    write_opt_in()?;
    let client = jmap::authenticated_client().await?;
    let label = format!("fastmail-cli-live-{}", uuid::Uuid::new_v4());
    let mut contact_ids = Vec::new();
    let mut note_id = None;
    let result: Result<()> = async {
        let created = client.create_contact_card(json!({"name":{"full":label},"addresses":{"home":{"full":"123 Test St"}},"anniversaries":{"birthday":{"kind":"birth","date":{"@type":"PartialDate","year":1990,"month":1,"day":2}}}}), None).await?;
        let id = created["created"]["new"]["id"].as_str().context("Missing created contact ID")?.to_owned(); contact_ids.push(id.clone());
        let card = client.contacts_get(false, Some(vec![id.clone()])).await?;
        let uid = card["list"][0]["uid"].as_str().context("Missing UID")?;
        let group = client.create_contact_card(json!({"kind":"group","name":{"full":format!("{label} group")},"members":{uid:true}}),None).await?;
        contact_ids.push(group["created"]["new"]["id"].as_str().context("Missing group ID")?.into());
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1,1).write_to(&mut png,image::ImageFormat::Png)?;
        client.set_contact_photo(&id,png.into_inner(),"image/png").await?;
        let card = client.contacts_get(false,Some(vec![id])).await?;
        ensure!(card["list"][0]["media"].as_object().is_some_and(|m| m.values().any(|v| v["kind"] == "photo")));
        let note = client.create_rich_note(NoteEdit { title:Some(label.clone()), body:Some("Plain alternative".into()), html_body:Some("<b>Rich content</b>".into()), attachments:Some(vec![jmap::AttachmentData { filename:"live.txt".into(),content_type:"text/plain".into(),content:jmap::AttachmentContent::Bytes(b"attachment".to_vec()) }]) }).await?;
        let id = note["id"].as_str().context("Missing note ID")?.to_owned(); note_id = Some(id.clone());
        let updated = client.update_rich_note(&id,NoteEdit { title:Some(format!("{label} updated")),..Default::default() }).await?;
        let id = updated["id"].as_str().context("Missing replacement ID")?.to_owned(); note_id = Some(id.clone());
        let note = client.get_note(&id).await?;
        ensure!(note["htmlBody"].as_array().is_some_and(|a| !a.is_empty()));
        ensure!(note["attachments"].as_array().is_some_and(|a| !a.is_empty()));
        Ok(())
    }.await;
    let mut cleanup = Ok(());
    for id in contact_ids {
        if let Err(error) = client.delete_jmap_contact(&id).await {
            cleanup = Err(error.into());
        }
    }
    if let Some(id) = note_id
        && let Err(error) = client.delete_note(&id).await
    {
        cleanup = Err(error);
    }
    result.and(cleanup)
}

#[tokio::test]
#[ignore = "creates/deletes a calendar, recurring event, and files; requires FASTMAIL_LIVE_WRITE_TESTS=1"]
async fn live_dav_files_recurrence_and_freebusy() -> Result<()> {
    write_opt_in()?;
    let (username, password) = Config::load()?.get_dav_credentials()?;
    let calendar = DavClient::new(
        "https://caldav.fastmail.com/",
        username.clone(),
        password.clone(),
    )?;
    let files = DavClient::new("https://myfiles.fastmail.com/", username, password)?;
    let label = format!("fastmail-cli-live-{}", uuid::Uuid::new_v4());
    let folder = format!("{}{label}/", calendar.calendar_home());
    let event = format!("{folder}event.ics");
    let file = format!("/{label}.txt");
    let copy = format!("/{label}-copy.txt");
    let moved = format!("/{label}-moved.txt");
    let mut created_calendar = false;
    let result: Result<()> = async {
        calendar.create_calendar(&folder,&label,None,None).await?; created_calendar = true;
        let data = format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//fastmail-cli//Live test//EN\r\nBEGIN:VEVENT\r\nUID:{label}\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20300101T120000Z\r\nDTEND:20300101T130000Z\r\nRRULE:FREQ=DAILY;COUNT=2\r\nSUMMARY:{label}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
        calendar.put(&event,data.into_bytes(),"text/calendar",None,false).await?;
        let events = calendar.events_expanded(&folder,Some("20300101T000000Z"),Some("20300104T000000Z"),true).await?;
        ensure!(events.iter().filter_map(|e| e.calendar_data.as_deref()).map(|s| s.matches("BEGIN:VEVENT").count()).sum::<usize>() >= 2,"Server did not expand recurrence instances");
        ensure!(calendar.free_busy(&folder,"20300101T000000Z","20300104T000000Z").await?.contains("VFREEBUSY"));
        files.put(&file,b"live content".to_vec(),"text/plain",None,false).await?;
        files.transfer(&file,&copy,true,false).await?; files.transfer(&copy,&moved,false,false).await?;
        ensure!(files.get(&moved).await?.0 == b"live content");
        Ok(())
    }.await;
    for path in [&file, &copy, &moved] {
        let _ = files.delete(path, None).await;
    }
    let cleanup = if created_calendar {
        calendar.delete(&folder, None).await
    } else {
        Ok(())
    };
    result.and(cleanup)
}

#[tokio::test]
#[ignore = "sends REQUEST/CANCEL to an explicitly supplied test recipient"]
async fn live_itip_scheduling() -> Result<()> {
    write_opt_in()?;
    ensure!(
        std::env::var("FASTMAIL_LIVE_SCHEDULING").as_deref() == Ok("1"),
        "Set FASTMAIL_LIVE_SCHEDULING=1 to send calendar test messages"
    );
    let originator = std::env::var("FASTMAIL_LIVE_ORIGINATOR")?;
    let recipient = std::env::var("FASTMAIL_LIVE_RECIPIENT")?;
    ensure!(
        !originator.contains(['\r', '\n']) && !recipient.contains(['\r', '\n']),
        "Invalid calendar address"
    );
    let (username, password) = Config::load()?.get_dav_credentials()?;
    let client = DavClient::new("https://caldav.fastmail.com/", username, password)?;
    let endpoints = client.scheduling().await?;
    let outbox = endpoints["outbox"][0]
        .as_str()
        .context("No scheduling outbox")?;
    let uid = uuid::Uuid::new_v4().to_string();
    let message = |method, sequence| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//fastmail-cli//Live test//EN\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nSEQUENCE:{sequence}\r\nDTSTAMP:20260101T000000Z\r\nDTSTART:20300101T120000Z\r\nDTEND:20300101T130000Z\r\nORGANIZER:{originator}\r\nATTENDEE:{recipient}\r\nSUMMARY:fastmail-cli scheduling test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    let sent = client
        .schedule(
            outbox,
            &originator,
            std::slice::from_ref(&recipient),
            message("REQUEST", 0).into_bytes(),
        )
        .await;
    let canceled = client
        .schedule(
            outbox,
            &originator,
            std::slice::from_ref(&recipient),
            message("CANCEL", 1).into_bytes(),
        )
        .await;
    for response in [sent?, canceled?] {
        ensure!(
            response["responses"]
                .as_array()
                .is_some_and(|r| r.iter().all(|r| r["requestStatus"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("2.")))),
            "Scheduling recipient failure: {response}"
        );
    }
    Ok(())
}
