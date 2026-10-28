//! Observed grab, dispense and return experiment. Never writes a body or prop pose.
use cc_render_plugin::cc_render::scene_water::WaterSnapshot;
use serde::Serialize;
use serde_json::{json, Value};
use super::prompt_recording::TimedPrompt;

const SHOWER_PROMPT: &str = "A person stands upright and looks forward under a running shower, gently rubbing their hands over their arms and torso to wash and rinse off.";

pub(super) enum Action { Goal(u64, u64, u8, Value) }

/// A bounded prompt experiment that keeps the physical bottle as its scene target.
#[derive(Serialize)]
struct TargetedSequence {
    interaction: String,
    prompt: TimedPrompt,
}

impl TargetedSequence {
    fn from_env(world: &Value, target: &str) -> anyhow::Result<Option<Self>> {
        let Ok(interaction) = std::env::var("CC_CONDO_CADDY_SEQUENCE") else { return Ok(None) };
        let declaration = world["office"]["objects"].as_array()
            .and_then(|objects| objects.iter().find(|o| o["id"] == target))
            .and_then(|object| object["interactions"].as_array())
            .and_then(|items| items.iter().find(|i| i["id"] == interaction))
            .ok_or_else(|| anyhow::anyhow!("sequence interaction is not declared on the bottle"))?;
        Self::new(interaction, declaration).map(Some)
    }

    fn new(interaction: String, declaration: &Value) -> anyhow::Result<Self> {
        let phases = declaration["phases"].as_array().ok_or_else(|| anyhow::anyhow!("missing phases"))?;
        anyhow::ensure!(phases.len() == 1, "sequence trial requires one semantic phase");
        let phase = &phases[0];
        anyhow::ensure!(["stand_at", "transition", "dispense"].iter().all(|k| phase[k].is_null())
            && phase["relations"].as_array().is_none_or(|r| r.is_empty()),
            "sequence trial cannot guide hands, navigate or change attachment");
        let description = phase["prompt"].as_str().ok_or_else(|| anyhow::anyhow!("missing prompt"))?;
        Ok(Self { interaction, prompt: TimedPrompt::new(description.into(), 12.)? })
    }
}

#[derive(Serialize)]
pub(super) struct CaddyTask {
    object_handling: &'static str,
    target: String,
    status: &'static str,
    home: [f64; 3],
    shelf_y: f64,
    lifted: bool,
    poured_liters: f64,
    left_palm_impacts: u64,
    pour_baseline: Option<(f64,u64)>,
    prompt_revision: Option<u64>,
    prompt_started_pts_ms: Option<u64>,
    released_pts_ms: Option<u64>,
    completed_pts_ms: Option<u64>,
    error: Option<String>,
    shower: TimedPrompt,
    sequence: Option<TargetedSequence>,
}

impl CaddyTask {
    pub fn from_env(world: &Value) -> anyhow::Result<Option<Self>> {
        let Ok(target) = std::env::var("CC_CONDO_CADDY_TASK") else { return Ok(None); };
        anyhow::ensure!(matches!(target.as_str(),"shampoo"|"conditioner"|"body_wash"), "unknown caddy bottle");
        let prop = world["props"].as_array().and_then(|props| props.iter().find(|p| p["id"] == target))
            .ok_or_else(|| anyhow::anyhow!("caddy target is not a loaded physical prop"))?;
        let home = std::array::from_fn(|i| prop["transform"][3][i].as_f64().unwrap());
        let sequence = TargetedSequence::from_env(world, &target)?;
        Ok(Some(Self { object_handling:"declared_interact_coupling",target,status:"waiting_for_arrival",home,
            shelf_y:prop["aabb_min"][1].as_f64().unwrap(), lifted:false, poured_liters:0.,
            left_palm_impacts:0, pour_baseline:None, prompt_revision:None,prompt_started_pts_ms:None,
            released_pts_ms:None,completed_pts_ms:None,error:None,
            shower:TimedPrompt::new(SHOWER_PROMPT.into(),30.)?,
            sequence }))
    }

    pub fn save(&self, path: &std::path::Path) -> anyhow::Result<()> {
        std::fs::write(path,serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn finished(&self) -> bool { matches!(self.status,"failed"|"complete"|"trial_complete") }

    fn fail(&mut self, error: String) { self.status="failed"; self.error=Some(error); }

    pub fn observe(&mut self, event: &Value, session: &str, navigation: &str,
        water: &[WaterSnapshot]) -> Option<Action> {
        if self.finished() { return None; }
        if matches!(event["kind"].as_str(),Some("motion_error"|"worker_error")) {
            self.fail(event["error"].to_string()); return None;
        }
        if event["kind"] == "motion_goal_status" && event["goal"]["session_id"] == session
            && event["status"] == "failed" {
            self.fail(event.to_string()); return None;
        }
        if self.status == "prompting_sequence" {
            let sequence = self.sequence.as_mut()?;
            let action = sequence.prompt.observe(event,session,navigation)
                .map(|(revision,pts,prompt)| Action::Goal(revision,pts,2,json!({"description":prompt})));
            if sequence.prompt.complete() {
                // A finished recording does not establish pouring or shelf placement.
                self.status = "trial_complete";
                self.completed_pts_ms = event["state"]["office"]["avatar"]["displayed_pts_ms"].as_u64();
            }
            return action;
        }
        if self.status == "showering" {
            let action=self.shower.observe(event,session,navigation)
                .map(|(revision,pts,prompt)| Action::Goal(revision,pts,2,json!({"description":prompt})));
            if self.shower.complete() {
                self.status="complete";
                self.completed_pts_ms=event["state"]["office"]["avatar"]["displayed_pts_ms"].as_u64();
            }
            return action;
        }
        if event["kind"] == "motion_goal_status" && event["goal"]["session_id"] == session {
            return self.goal_status(event,navigation);
        }
        if event["kind"] != "scene_state" { return None; }
        self.observe_world(&event["state"], session, water)
    }

    fn observe_world(&mut self, world: &Value, session: &str, water: &[WaterSnapshot]) -> Option<Action> {
        let avatar=&world["office"]["avatar"];
        if avatar["native_stream"]["session_id"] != session || avatar["ready"] != true { return None; }
        let pts=avatar["displayed_pts_ms"].as_u64()?;
        if avatar["native_stream"]["generation_revision"].as_u64() != self.prompt_revision { return None; }
        self.prompt_started_pts_ms.get_or_insert(pts);
        let prop=world["props"].as_array()?.iter().find(|p| p["id"] == self.target)?;
        if self.status == "pouring" {
            if let Some(liquid)=water.iter().find(|w| w.id == format!("{}_liquid",self.target)) {
                let base=*self.pour_baseline.get_or_insert((liquid.total_liters,liquid.near_palm_impacts[0]));
                self.poured_liters=(liquid.total_liters-base.0).max(0.);
                self.left_palm_impacts=liquid.near_palm_impacts[0].saturating_sub(base.1);
            }
        }
        let held=world["office"]["couplings"].as_array()?.iter().any(|c|
            c["object"] == self.target && c["release"].is_null()
                && c["anchors"].as_array().is_some_and(|a| a.iter().any(|a| a[0] == "right")));
        let near=self.at_shelf(prop);
        self.lifted |= held && !near;
        self.advance_task(pts,held,near)
    }

    fn advance_task(&mut self, pts:u64, held:bool, near:bool) -> Option<Action> {
        if self.status == "awaiting_attachment" {
            if held { return self.begin_bottle_use(pts); }
            if pts.saturating_sub(self.prompt_started_pts_ms?) > 3000 {
                self.fail("pickup completed without an observed bottle attachment".into());
            }
            return None;
        }
        if self.status == "pouring" { return self.observe_pour(pts,held); }
        if self.status == "awaiting_release" {
            if !held && near { self.status="settling"; self.released_pts_ms=Some(pts); }
            else if pts.saturating_sub(self.released_pts_ms?) > 3000 {
                self.fail("set_down completed without observed shelf release".into());
            }
        } else if self.status == "settling" {
            if !near || held { self.fail("declared set_down did not leave the bottle upright and released on its shelf".into()); }
            else if pts.saturating_sub(self.released_pts_ms?) >= 1000 {
                if self.error.is_some() { self.status="failed"; return None; }
                self.status="turning_outwards";
                let revision=self.prompt_revision?+1;
                self.prompt_revision=Some(revision);
                return Some(Action::Goal(revision,pts,3,json!({"operation":"interact",
                    "target_id":"shower_caddy","interaction":"face_outwards"})));
            }
        }
        None
    }

    fn begin_bottle_use(&mut self, pts:u64) -> Option<Action> {
        let revision=self.prompt_revision?+1;
        if let Some(sequence) = self.sequence.as_mut() {
            self.status = "prompting_sequence";
            sequence.prompt.start(revision,pts);
            self.prompt_revision = Some(revision);
            return Some(Action::Goal(revision,pts,3,json!({"operation":"interact",
                "target_id":self.target,"interaction":sequence.interaction})));
        }
        self.status="pouring";
        self.prompt_revision=Some(revision);
        self.prompt_started_pts_ms=None;
        Some(Action::Goal(revision,pts,3,json!({"operation":"interact",
            "target_id":self.target,"interaction":"pour_into_hand"})))
    }

    fn observe_pour(&mut self, pts:u64, held:bool) -> Option<Action> {
        if !held { self.fail("declared bottle attachment lost before set_down".into()); }
        else if self.lifted && self.poured_liters >= 0.0001 && self.left_palm_impacts >= 3 {
            return self.return_bottle(pts);
        } else if pts.saturating_sub(self.prompt_started_pts_ms?) > 10000 {
            self.error=Some("10 seconds elapsed without observed dispensing into the palm region".into());
            return self.return_bottle(pts);
        }
        None
    }

    fn return_bottle(&mut self, pts:u64) -> Option<Action> {
        self.status="returning";
        Some(Action::Goal(self.prompt_revision?+1,pts,3,
            json!({"operation":"interact","target_id":self.target,"interaction":"set_down"})))
    }

    fn goal_status(&mut self, event: &Value, navigation: &str) -> Option<Action> {
        if event["status"] == "failed" { self.fail(event.to_string()); return None; }
        if event["status"] != "completed" { return None; }
        let goal=&event["goal"]["goal"];
        let pts=event["displayed_pts_ms"].as_u64()?;
        let revision=event["goal"]["generation_revision"].as_u64()?+1;
        if self.status == "turning_outwards" && goal["operation"] == "interact"
            && goal["target_id"] == "shower_caddy" && goal["interaction"] == "face_outwards"
            && Some(revision-1) == self.prompt_revision {
            self.status="showering";
            let (revision,pts,prompt)=self.shower.start(revision,pts);
            return Some(Action::Goal(revision,pts,2,json!({"description":prompt})));
        }
        if self.status == "waiting_for_arrival" && goal["operation"] == "navigate" && goal["target_id"] == navigation {
            self.status="picking_up";
            return Some(Action::Goal(revision,pts,3,json!({"operation":"interact","target_id":self.target,"interaction":"pick_up"})));
        }
        if goal["operation"] != "interact" || goal["target_id"] != self.target { return None; }
        if self.status == "picking_up" && goal["interaction"] == "pick_up" {
            self.status="awaiting_attachment"; self.prompt_revision=Some(revision-1);
            self.prompt_started_pts_ms=Some(pts);
            return None;
        }
        if self.status == "returning" && goal["interaction"] == "set_down" {
            if revision-1 <= self.prompt_revision? { return None; }
            self.status="awaiting_release"; self.prompt_revision=Some(revision-1);
            self.released_pts_ms=Some(pts);
        }
        None
    }

    fn at_shelf(&self, prop: &Value) -> bool {
        let distance=(0..3).map(|i| (prop["transform"][3][i].as_f64().unwrap_or(f64::INFINITY)-self.home[i]).powi(2)).sum::<f64>().sqrt();
        distance < 0.05 && prop["transform"][1][1].as_f64().is_some_and(|up| up > 0.97)
            && prop["aabb_min"][1].as_f64().is_some_and(|y| (y-self.shelf_y).abs() < 0.012)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn task() -> CaddyTask {
        CaddyTask { object_handling:"declared_interact_coupling",target:"shampoo".into(),status:"picking_up",home:[0.,1.,0.],shelf_y:0.9,
            lifted:false,poured_liters:0.,left_palm_impacts:0,pour_baseline:None,prompt_revision:None,
            prompt_started_pts_ms:None,released_pts_ms:None,completed_pts_ms:None,error:None,
            shower:TimedPrompt::new(SHOWER_PROMPT.into(),30.).unwrap(), sequence:None }
    }

    #[test]
    fn caddy_pour_ignores_prephase_liquid_and_waits_for_new_palm_impacts() {
        let mut t=task(); t.status="pouring"; t.prompt_revision=Some(5);
        let mut water=WaterSnapshot { id:"shampoo_liquid".into(),revision:0,flow:1.,liters_per_minute:0.3,
            total_liters:0.02,near_palm_impacts:[30,0],visible_particles:0,runoff_particles:0,body_impacts:30,
            drain_impacts:0,dropped_emissions:0,unresolved_collisions:0,collision_meshes:0,
            collision_triangles:0,collision_revision:0,collision_error:None,collision_update_ms:0.,
            simulation_ms:0.,degenerate_triangles:0 };
        let state=|pts,revision| json!({"kind":"scene_state","state":{"props":[{"id":"shampoo",
            "transform":[[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,1.2,0,1]],"aabb_min":[-0.03,1.1,-0.03]}],
            "office":{"couplings":[{"object":"shampoo","anchors":[["right",[0,0,0],"power"]]}],
                "avatar":{"ready":true,"displayed_pts_ms":pts,"native_stream":{"session_id":"test","generation_revision":revision}}}}});
        assert!(t.observe(&state(1000,4),"test","shower_screen",&[water.clone()]).is_none());
        assert!(t.pour_baseline.is_none(), "queued pouring is not displayed pouring");
        assert!(t.observe(&state(1100,5),"test","shower_screen",&[water.clone()]).is_none());
        assert_eq!(t.left_palm_impacts,0);
        assert_eq!(t.poured_liters,0.);
        water.total_liters+=0.001;
        water.near_palm_impacts[0]+=2;
        assert!(t.observe(&state(1200,5),"test","shower_screen",&[water.clone()]).is_none());
        water.near_palm_impacts[0]+=1;
        assert!(matches!(t.observe(&state(1300,5),"test","shower_screen",&[water]),Some(Action::Goal(6,1300,3,_))));
        assert_eq!(t.status,"returning");
    }

    #[test]
    fn caddy_interact_waits_for_pickup_pour_and_observed_placement() {
        let mut t=task();
        let event=|target,interaction| json!({"status":"completed","displayed_pts_ms":5000,
            "goal":{"generation_revision":4,"goal":{"operation":"interact","target_id":target,"interaction":interaction}}});
        assert!(t.goal_status(&event("conditioner","pick_up"),"shower_screen").is_none());
        assert!(t.goal_status(&event("shampoo","set_down"),"shower_screen").is_none());
        assert!(t.goal_status(&event("shampoo","pick_up"),"shower_screen").is_none());
        assert_eq!(t.status,"awaiting_attachment");
        assert!(t.advance_task(5010,false,true).is_none(), "completion precedes physical attachment");
        assert!(matches!(t.advance_task(5050,true,true),Some(Action::Goal(5,5050,3,_))),
            "an attached bottle may still be on the shelf when the next motion starts");
        assert!(!t.lifted, "attachment alone does not prove movement away from the shelf");
        assert!(t.goal_status(&event("shampoo","pick_up"),"shower_screen").is_none());
        let state=|pts| json!({"kind":"scene_state","state":{"props":[{"id":"shampoo",
            "transform":[[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,1,0,1]],"aabb_min":[-0.03,0.9,-0.03]}],
            "office":{"couplings":[{"object":"shampoo","anchors":[["right",[0,0,0],"power"]]}],"avatar":{"ready":true,"displayed_pts_ms":pts,
                "native_stream":{"session_id":"test","generation_revision":5}}}}});
        assert!(t.observe(&state(6000),"test","shower_screen",&[]).is_none());
        assert!(matches!(t.observe(&state(37001),"test","shower_screen",&[]),Some(Action::Goal(_,_,3,_))));
        assert_eq!(t.status,"returning", "a failed pour still attempts to put the bottle back");
        assert!(t.error.is_some());
        assert!(t.released_pts_ms.is_none());
        t.status="pouring";
        t.error=None;
        t.lifted=true;
        t.poured_liters=0.002;
        assert!(t.advance_task(12000,true,true).is_none(), "pouring away from palm cannot pass");
        t.left_palm_impacts=3;
        let Some(Action::Goal(6,12100,3,goal))=t.advance_task(12100,true,false) else { panic!("pour should request set_down") };
        assert_eq!(goal,json!({"operation":"interact","target_id":"shampoo","interaction":"set_down"}));
        assert_eq!(t.status,"returning");
        assert!(t.goal_status(&event("shampoo","set_down"),"shower_screen").is_none());
        assert_eq!(t.status,"returning", "stale completion cannot release a newer interaction");
        let mut placed=event("shampoo","set_down");
        placed["goal"]["generation_revision"]=json!(6);
        placed["displayed_pts_ms"]=json!(13000);
        t.goal_status(&placed,"shower_screen");
        assert_eq!(t.status,"awaiting_release");
        t.advance_task(13100,true,false);
        assert_eq!(t.status,"awaiting_release", "completion event alone is not a released bottle");
        assert!(t.advance_task(13200,false,true).is_none());
        assert_eq!(t.status,"settling");
        let Some(Action::Goal(7,14200,3,goal))=t.advance_task(14200,false,true) else { panic!("settled bottle should permit turning") };
        assert_eq!(goal,json!({"operation":"interact","target_id":"shower_caddy","interaction":"face_outwards"}));
        assert_eq!(t.status,"turning_outwards");
        assert!(t.completed_pts_ms.is_none());
        let mut turned=event("shower_caddy","face_outwards");
        assert!(t.goal_status(&turned,"shower_screen").is_none(), "stale turns cannot start washing");
        turned["goal"]["generation_revision"]=json!(7);
        assert!(matches!(t.goal_status(&turned,"shower_screen"),Some(Action::Goal(8,_,2,_))));
        assert_eq!(t.status,"showering");
        assert!(t.goal_status(&turned,"shower_screen").is_none());
        let washing=|revision,pts| json!({"kind":"scene_state","state":{"office":{"avatar":{
            "ready":true,"displayed_pts_ms":pts,"native_stream":{"session_id":"test","generation_revision":revision}}}}});
        assert!(t.observe(&washing(7,20000),"test","shower_screen",&[]).is_none());
        assert!(t.observe(&washing(8,21000),"test","shower_screen",&[]).is_none());
        assert!(t.observe(&washing(8,50999),"test","shower_screen",&[]).is_none());
        assert!(matches!(t.observe(&washing(8,51000),"test","shower_screen",&[]),Some(Action::Goal(9,51000,2,_))));
        assert_eq!(t.status,"showering", "generated stop does not prove it was displayed");
        t.observe(&washing(9,51600),"test","shower_screen",&[]);
        assert_eq!(t.status,"complete");
        assert_eq!(t.completed_pts_ms,Some(51600));
        t.status="settling";
        t.error=Some("pour not observed".into());
        t.advance_task(14250,false,true);
        assert_eq!(t.status,"failed", "successful return cannot erase a failed pour");
        t.status="settling";
        t.advance_task(12500,false,false);
        assert_eq!(t.status,"failed", "leaving the shelf during settling must fail immediately");
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;
    use super::tests::task;

    #[test]
    fn targeted_sequence_waits_for_attachment_and_preserves_target() {
        let mut t = task();
        let description = "grab a bottle from the shelf, pour some into your hand, and place it back";
        let declaration = json!({"phases":[{"prompt":description}]});
        t.sequence = Some(TargetedSequence::new("use_and_return".into(), &declaration).unwrap());
        t.status = "awaiting_attachment";
        t.prompt_revision = Some(3);
        t.prompt_started_pts_ms = Some(1000);
        assert!(t.advance_task(1100,false,true).is_none());
        let Some(Action::Goal(revision,_,version,goal)) = t.advance_task(1200,true,true) else { panic!("missing targeted goal") };
        assert_eq!((revision,version),(4,3));
        assert_eq!(goal,json!({"operation":"interact","target_id":"shampoo","interaction":"use_and_return"}));
        assert_eq!(t.status,"prompting_sequence");
        let state = |pts,revision| json!({"kind":"scene_state","state":{"office":{"avatar":{
            "ready":true,"displayed_pts_ms":pts,"native_stream":{"session_id":"test","generation_revision":revision}}}}});
        assert!(t.observe(&state(2000,4),"test","shampoo",&[]).is_none());
        assert!(matches!(t.observe(&state(14000,4),"test","shampoo",&[]),Some(Action::Goal(5,14000,2,_))));
        t.observe(&state(14500,5),"test","shampoo",&[]);
        assert_eq!(t.status,"trial_complete", "recording duration is not manipulation success");
        assert!(t.finished());
    }

    #[test]
    fn targeted_sequence_rejects_conditioned_or_attachment_changing_phases() {
        for (key,value) in [("stand_at",json!({"zone":"approach"})),
            ("transition",json!({"kind":"release"})), ("dispense",json!({"outlet":"outlet"})),
            ("relations",json!([{"kind":"touch"}]))] {
            let mut declaration = json!({"phases":[{"prompt":"Use the bottle."}]});
            declaration["phases"][0][key] = value;
            assert!(TargetedSequence::new("use_and_return".into(), &declaration).is_err());
        }
    }

}
