use hyve::prelude::*;

pub struct Db;

#[server(POST "/projects")]
pub async fn create(db: Ctx<Db>) -> HyveResult<()> {
    let _ = db;
    Ok(())
}

fn main() {}
