// This file is released under the same terms as Rust itself.

use db::Builder;
use rusqlite::{self, Connection};
use postgres::{ Connection as PgConnection, SslMode};
use postgres::IntoConnectParams;
use std::convert::AsRef;
use std::error::Error;
use std::path::Path;
use ui::github::TeamId;
use pipeline::PipelineId;

pub fn from_builder(builder: &Builder)
        -> Result<Cache, Box<Error + Send + Sync>> {
    Ok(match *builder {
        Builder::Sqlite(ref c) => Cache::Sqlite(try!(Sqlite::open(c))),
        Builder::Postgres(ref c) => Cache::Postgres(try!(Postgres::open(c.clone()))),
    })
}

pub enum Cache {
    Sqlite(Sqlite),
    Postgres(Postgres),
}

impl Cache {
    pub fn set_teams_with_write<T: Iterator<Item=TeamId>>(
        &mut self,
        pipeline_id: PipelineId,
        teams: T,
    ) {
        match *self {
            Cache::Sqlite(ref mut c) => c.set_teams_with_write(pipeline_id, teams),
            Cache::Postgres(ref mut c) => c.set_teams_with_write(pipeline_id, teams),
        }
    }
    pub fn teams_with_write(&mut self, pipeline_id: PipelineId) -> Vec<TeamId> {
        match *self {
            Cache::Sqlite(ref mut c) => c.teams_with_write(pipeline_id),
            Cache::Postgres(ref mut c) => c.teams_with_write(pipeline_id),
        }
    }
    pub fn is_org(&mut self, pipeline_id: PipelineId) -> Option<bool> {
        match *self {
            Cache::Sqlite(ref mut c) => c.is_org(pipeline_id),
            Cache::Postgres(ref mut c) => c.is_org(pipeline_id),
        }
    }
    pub fn set_is_org(&mut self, pipeline_id: PipelineId, is_org: bool) {
        match *self {
            Cache::Sqlite(ref mut c) => c.set_is_org(pipeline_id, is_org),
            Cache::Postgres(ref mut c) => c.set_is_org(pipeline_id, is_org),
        }
    }
}

pub struct Sqlite {
    conn: Connection,
}

impl Sqlite {
    fn open<Q: AsRef<Path>>(path: Q) -> rusqlite::Result<Self> {
        let conn = try!(Connection::open(path));
        try!(conn.execute_batch(r###"
            CREATE TABLE IF NOT EXISTS github_teams_with_write (
                pipeline_id INTEGER,
                team_id INTEGER,
                UNIQUE (pipeline_id, team_id)
            );
            CREATE TABLE IF NOT EXISTS github_is_org (
                pipeline_id INTEGER PUBLIC KEY,
                is_org BOOLEAN
            )
        "###));
        Ok(Sqlite{
            conn: conn,
        })
    }
    fn set_teams_with_write<T: Iterator<Item=TeamId>>(
        &mut self,
        pipeline_id: PipelineId,
        team_ids: T,
    ) {
        let trans = self.conn
            .transaction()
            .expect("Start pop-from-queue transaction");
        let sql = r###"
            DELETE FROM github_teams_with_write
            WHERE pipeline_id = ?;
        "###;
        trans.execute(sql, &[
            &pipeline_id.0,
        ]).expect("to clear all teams");
        let sql = r###"
            REPLACE INTO github_teams_with_write (pipeline_id, team_id)
            VALUES (?, ?);
        "###;
        for team_id in team_ids {
            assert!(team_id.0 != 0);
            trans.execute(sql, &[
                &pipeline_id.0,
                &(team_id.0 as i32),
            ]).expect("to add a team");
        }
        trans.commit().expect("Commit pop-from-queue transaction");
    }
    fn teams_with_write(
        &mut self,
        pipeline_id: PipelineId
    ) -> Vec<TeamId> {
        let sql = r###"
            SELECT team_id
            FROM github_teams_with_write
            WHERE pipeline_id = ?;
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let rows = stmt
            .query_map(&[&pipeline_id.0], |row| {
                TeamId(row.get::<_, i32>(0) as u32)
            })
            .expect("Get running entry");
        let ret_val = rows
            .map(|team_id| team_id.expect("to be connected"))
            .collect();
        ret_val
    }
    fn is_org(&mut self, pipeline_id: PipelineId) -> Option<bool> {
        let sql = r###"
            SELECT is_org
            FROM github_is_org
            WHERE pipeline_id = ?;
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let mut rows = stmt
            .query_map(&[&pipeline_id.0], |row| row.get::<_, bool>(0))
            .expect("Get is-org");
        rows.next().map(|row| row.expect("SQLite to work"))
    }
    fn set_is_org(&mut self, pipeline_id: PipelineId, is_org: bool) {
        let sql = r###"
            REPLACE INTO github_is_org (pipeline_id, is_org)
            VALUES (?, ?);
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0, &is_org
        ]).expect("to set is_org");
    }
}

pub struct Postgres {
    conn: PgConnection,
}

impl Postgres {
    fn open<Q: IntoConnectParams>(params: Q) -> Result<Self, Box<Error + Send + Sync>> {
        let conn = try!(PgConnection::connect(params, SslMode::None));
        try!(conn.batch_execute(r###"
            CREATE TABLE IF NOT EXISTS github_teams_with_write (
                pipeline_id INTEGER,
                team_id INTEGER,
                UNIQUE (pipeline_id, team_id)
            );
            CREATE TABLE IF NOT EXISTS github_is_org (
                pipeline_id INTEGER PUBLIC KEY,
                is_org BOOLEAN
            )
        "###));
        Ok(Postgres{
            conn: conn,
        })
    }
    fn set_teams_with_write<T: Iterator<Item=TeamId>>(
        &mut self,
        pipeline_id: PipelineId,
        team_ids: T,
    ) {
        let trans = self.conn
            .transaction()
            .expect("Start pop-from-queue transaction");
        let sql = r###"
            DELETE FROM github_teams_with_write
            WHERE pipeline_id = ?;
        "###;
        trans.execute(sql, &[
            &pipeline_id.0,
        ]).expect("to clear all teams");
        let sql = r###"
            REPLACE INTO github_teams_with_write (pipeline_id, team_id)
            VALUES (?, ?);
        "###;
        for team_id in team_ids {
            assert!(team_id.0 != 0);
            trans.execute(sql, &[
                &pipeline_id.0,
                &(team_id.0 as i32),
            ]).expect("to add a team");
        }
        trans.commit().expect("Commit pop-from-queue transaction");
    }
    fn teams_with_write(
        &mut self,
        pipeline_id: PipelineId
    ) -> Vec<TeamId> {
        let sql = r###"
            SELECT team_id
            FROM github_teams_with_write
            WHERE pipeline_id = ?;
        "###;
        let stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let rows = stmt.query(&[&pipeline_id.0]);
        let rows = rows.expect("Get running entry");
        let rows = rows.iter();
        let rows = rows.map(|row| {
            TeamId(row.get::<_, i32>(0) as u32)
        });
        let ret_val = rows.collect();
        ret_val
    }
    fn is_org(&mut self, pipeline_id: PipelineId) -> Option<bool> {
        let sql = r###"
            SELECT is_org
            FROM github_is_org
            WHERE pipeline_id = ?;
        "###;
        let stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let rows = stmt.query(&[&pipeline_id.0]);
        let rows = rows.expect("Get is-org");
        let rows = rows.iter();
        let mut rows = rows.map(|row| row.get::<_, bool>(0));
        rows.next()
    }
    fn set_is_org(&mut self, pipeline_id: PipelineId, is_org: bool) {
        let sql = r###"
            REPLACE INTO github_is_org (pipeline_id, is_org)
            VALUES (?, ?);
        "###;
        self.conn.execute(sql, &[
            &pipeline_id.0, &is_org
        ]).expect("to set is_org");
    }
}