// This file is released under the same terms as Rust itself.

use rusqlite::{self, Connection};
use std::convert::AsRef;
use std::path::Path;
use ui::github::TeamId;
use pipeline::PipelineId;

pub trait Cache {
	fn set_teams_with_write<T: Iterator<Item=TeamId>>(&mut self, PipelineId, T);
	fn teams_with_write(&mut self, PipelineId) -> Vec<TeamId>;
	fn team_has_write(&mut self, PipelineId, TeamId) -> bool;
    fn is_org(&mut self, PipelineId) -> Option<bool>;
    fn set_is_org(&mut self, PipelineId, bool);
}

pub struct SqliteCache {
    conn: Connection,
}

impl SqliteCache {
    pub fn open<Q: AsRef<Path>>(path: Q) -> rusqlite::Result<Self> {
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
        Ok(SqliteCache{
            conn: conn,
        })
    }
}

impl Cache for SqliteCache {
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
    fn team_has_write(
        &mut self,
        pipeline_id: PipelineId,
        team_id: TeamId,
    ) -> bool {
        let sql = r###"
            SELECT pipeline_id
            FROM github_teams_with_write
            WHERE pipeline_id = ? AND team_id = ?;
        "###;
        let mut stmt = self.conn.prepare(&sql)
            .expect("Prepare peek-running query");
        let mut rows = stmt
            .query_map(&[&pipeline_id.0, &(team_id.0 as i32)], |_| ())
            .expect("Get running entry");
        rows.next().is_some()
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