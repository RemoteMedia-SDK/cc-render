//! One applied atmosphere drives all decorative scene systems; no pose access.
use super::weather::{CondoWeather, WeatherLight};
use crate::cc_render::scene_weather::{WeatherBridge, WeatherRequest, WeatherSnapshot};
use bevy::{prelude::*, render::camera::Exposure};

#[derive(Clone, Copy)]
pub(super) struct Atmosphere {
    pub sky: Vec4,
    pub lighting: WeatherLight,
    pub rain: f32,
    pub snow: f32,
}

impl Atmosphere {
    fn preset(preset: CondoWeather) -> Self {
        Self { sky: preset.sky(), lighting: preset.lighting(),
               rain: f32::from(preset == CondoWeather::Rain),
               snow: f32::from(preset == CondoWeather::Snow) }
    }

    fn mix(self, target: Self, amount: f32) -> Self {
        if amount <= 0.0 { return self; }
        if amount >= 1.0 { return target; }
        let a = self.lighting;
        let b = target.lighting;
        Self {
            sky: self.sky.lerp(target.sky, amount),
            rain: self.rain.lerp(target.rain, amount),
            snow: self.snow.lerp(target.snow, amount),
            lighting: WeatherLight {
                sun: mix_light(a.sun, b.sun, amount),
                ambient: mix_light(a.ambient, b.ambient, amount),
                environment: mix_light(a.environment, b.environment, amount),
                exposure: a.exposure.lerp(b.exposure, amount),
                window_lumens: a.window_lumens.lerp(b.window_lumens, amount),
                fog_start: a.fog_start.lerp(b.fog_start, amount),
                fog_end: a.fog_end.lerp(b.fog_end, amount),
                fog_color: Color::LinearRgba(a.fog_color.to_linear().mix(&b.fog_color.to_linear(), amount)),
            },
        }
    }
}

fn mix_light(a: f32, b: f32, amount: f32) -> f32 {
    // Blend brightness perceptually across daylight/night orders of magnitude.
    ((a + 1.0).ln().lerp((b + 1.0).ln(), amount)).exp() - 1.0
}

#[derive(Resource)]
pub(super) struct AppliedWeather {
    pub current: Atmosphere,
    pub surface_wetness: f32,
    from: Atmosphere,
    to: Atmosphere,
    target: CondoWeather,
    request_id: u64,
    duration: f32,
    elapsed: f32,
    wind_time: f64,
    exposure_override: Option<f32>,
}

impl AppliedWeather {
    fn new(preset: CondoWeather) -> Self {
        let atmosphere = Atmosphere::preset(preset);
        Self { current: atmosphere, surface_wetness: atmosphere.rain, from: atmosphere, to: atmosphere, target: preset,
               request_id: 0, duration: 0.0, elapsed: 0.0, wind_time: 0.0,
               exposure_override: None }
    }

    fn request(&mut self, request: WeatherRequest) {
        // Supersession starts at the displayed intermediate values, never at
        // the previous preset. Do not reset the integrated wind phase.
        if request.id <= self.request_id { return; }
        self.from = self.current;
        self.to = Atmosphere::preset(request.target);
        self.target = request.target;
        self.request_id = request.id;
        self.duration = request.seconds;
        self.elapsed = 0.0;
    }

    fn progress(&self) -> f32 {
        if self.duration == 0.0 { 1.0 } else { (self.elapsed / self.duration).min(1.0) }
    }

    fn step(&mut self, delta: f32) {
        let old_wind = self.current.sky.w;
        self.elapsed = (self.elapsed + delta).min(self.duration);
        let t = self.progress();
        self.current = self.from.mix(self.to, t * t * (3.0 - 2.0 * t));
        // Cosmetic water film: wets quickly, dries slowly, and survives retargets.
        let seconds = if self.current.rain > self.surface_wetness { 12.0 } else { 90.0 };
        self.surface_wetness += (self.current.rain - self.surface_wetness)
            * (1.0 - (-delta / seconds).exp());
        self.wind_time += f64::from((old_wind + self.current.sky.w) * 0.5 * delta);
        self.current.sky.x = self.wind_time as f32;
        if let Some(exposure) = self.exposure_override {
            self.current.lighting.exposure = exposure;
        }
    }

    fn snapshot(&self, elapsed_seconds: f64) -> WeatherSnapshot {
        WeatherSnapshot {
            request_id: self.request_id, target: self.target, progress: self.progress(), elapsed_seconds,
            cloud_cover: self.current.sky.y, night: self.current.sky.z,
            wind: self.current.sky.w, wind_time: self.current.sky.x,
            rain: self.current.rain, snow: self.current.snow,
            surface_wetness: self.surface_wetness,
            sun_illuminance: self.current.lighting.sun,
            environment_intensity: self.current.lighting.environment,
            exposure_ev100: self.current.lighting.exposure,
            fog_start: self.current.lighting.fog_start, fog_end: self.current.lighting.fog_end,
        }
    }
}

#[derive(Component)]
pub(super) struct WeatherSun;
#[derive(Component)]
pub(super) struct WindowSkylight;
#[derive(Component)]
pub(super) struct NightSpot(pub f32);
#[derive(Component)]
pub(super) struct BakedSkyStrength(pub f32);
#[derive(Resource, Default)]
pub(super) struct NightEmission(pub Vec<(Handle<StandardMaterial>, LinearRgba)>);

pub(super) fn install(app: &mut App) {
    let preset = CondoWeather::from_env().expect("weather validated at startup");
    let mut applied = AppliedWeather::new(preset);
    applied.exposure_override = std::env::var("CC_CONDO_EV100").ok()
        .and_then(|value| value.parse::<f32>().ok()).filter(|value| value.is_finite())
        .map(|value| value.clamp(6.0, 18.0));
    app.insert_resource(applied).init_resource::<NightEmission>()
        .add_systems(PreUpdate, advance)
        .add_systems(Update, (apply_lighting, apply_materials));
}

fn advance(time: Res<Time>, mut bridge: ResMut<WeatherBridge>, mut applied: ResMut<AppliedWeather>) {
    if bridge.requests.has_changed().unwrap_or(false) {
        let request = *bridge.requests.borrow_and_update();
        if let Some(request) = request { applied.request(request); }
    }
    applied.step(time.delta_secs());
    bridge.snapshots.send_replace(Some(applied.snapshot(time.elapsed_secs_f64())));
}

fn apply_lighting(
    applied: Res<AppliedWeather>, mut ambient: ResMut<AmbientLight>,
    mut sun: Query<&mut DirectionalLight, With<WeatherSun>>,
    mut cameras: Query<(&mut Exposure, &mut DistanceFog, &mut EnvironmentMapLight)>,
    mut windows: Query<&mut SpotLight, (With<WindowSkylight>, Without<NightSpot>)>,
    mut night_lights: Query<(&NightSpot, &mut SpotLight, &mut Visibility), Without<WindowSkylight>>,
) {
    let lighting = applied.current.lighting;
    ambient.brightness = lighting.ambient;
    for mut sun in &mut sun { sun.illuminance = lighting.sun; }
    for (mut exposure, mut fog, mut environment) in &mut cameras {
        exposure.ev100 = lighting.exposure;
        fog.color = lighting.fog_color;
        fog.falloff = FogFalloff::Linear { start: lighting.fog_start, end: lighting.fog_end };
        environment.intensity = lighting.environment;
    }
    for mut window in &mut windows { window.intensity = lighting.window_lumens; }
    for (night, mut spot, mut visibility) in &mut night_lights {
        spot.intensity = night.0 * applied.current.sky.z;
        *visibility = if spot.intensity > 0.0 { Visibility::Inherited } else { Visibility::Hidden };
    }
}

fn apply_materials(
    applied: Res<AppliedWeather>, emission: Res<NightEmission>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    baked: Query<(&BakedSkyStrength, &MeshMaterial3d<StandardMaterial>)>,
) {
    for (handle, colour) in &emission.0 {
        if let Some(material) = materials.get_mut(handle) {
            material.emissive = *colour * applied.current.sky.z;
        }
    }
    for (strength, handle) in &baked {
        if let Some(material) = materials.get_mut(&handle.0) {
            material.lightmap_exposure = strength.0 * applied.current.lighting.environment;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exterior_film_wets_and_dries_without_resetting_on_weather_requests() {
        let mut state = AppliedWeather::new(CondoWeather::Clear);
        assert_eq!(state.surface_wetness, 0.0);
        state.request(WeatherRequest::new(1, CondoWeather::Rain, 0.0).unwrap());
        state.step(0.0);
        assert_eq!(state.surface_wetness, 0.0);
        state.step(12.0);
        let wet = state.surface_wetness;
        assert!((wet - (1.0 - (-1.0_f32).exp())).abs() < 1e-6);
        state.request(WeatherRequest::new(2, CondoWeather::Night, 0.0).unwrap());
        state.step(0.0);
        assert_eq!(state.surface_wetness, wet);
        state.step(90.0);
        assert!((state.surface_wetness - wet * (-1.0_f32).exp()).abs() < 1e-6);
        assert_eq!(state.snapshot(102.0).surface_wetness, state.surface_wetness);
        assert_eq!(AppliedWeather::new(CondoWeather::Rain).surface_wetness, 1.0);
    }
    #[test]
    fn transitions_reach_endpoints_and_retarget_without_a_value_jump() {
        let mut state = AppliedWeather::new(CondoWeather::Clear);
        state.request(WeatherRequest::new(1, CondoWeather::Rain, 4.0).unwrap());
        state.step(2.0);
        let middle = state.current;
        assert!((middle.rain - 0.5).abs() < 1e-6);
        assert!(middle.lighting.sun > 650.0 && middle.lighting.sun < 14000.0);
        let phase = state.wind_time;
        state.request(WeatherRequest::new(2, CondoWeather::Night, 2.0).unwrap());
        state.step(0.0);
        assert_eq!(state.current.sky, middle.sky);
        assert_eq!(state.current.lighting.sun, middle.lighting.sun);
        assert_eq!(state.wind_time, phase);
        state.step(2.0);
        assert_eq!(state.current.lighting.sun, CondoWeather::Night.lighting().sun);
        assert_eq!(state.current.rain, 0.0);
        assert_eq!(state.current.sky.z, 1.0);
        assert_eq!(state.progress(), 1.0);
        state.request(WeatherRequest::new(1, CondoWeather::Clear, 0.0).unwrap());
        assert_eq!(state.target, CondoWeather::Night, "stale requests cannot replace newer weather");
    }

    #[test]
    fn wind_phase_integrates_and_explicit_exposure_survives_transitions() {
        let mut state = AppliedWeather::new(CondoWeather::Clear);
        state.exposure_override = Some(9.0);
        state.step(2.0);
        assert_eq!(state.current.sky.x, 2.0);
        state.request(WeatherRequest::new(1, CondoWeather::Snow, 0.0).unwrap());
        state.step(0.0);
        assert_eq!(state.current.sky.x, 2.0);
        state.step(2.0);
        assert!((state.current.sky.x - 3.2).abs() < 1e-5);
        assert_eq!(state.current.lighting.exposure, 9.0);
        assert_eq!(state.current.snow, 1.0);
    }

    #[test]
    fn atmosphere_updates_lights_fog_exposure_emission_and_baked_sky_together() {
        let mut app = App::new();
        let mut materials = Assets::<StandardMaterial>::default();
        let handle = materials.add(StandardMaterial::default());
        app.insert_resource(materials)
            .insert_resource(AmbientLight::default())
            .insert_resource(AppliedWeather::new(CondoWeather::Clear))
            .insert_resource(NightEmission(vec![(handle.clone(), LinearRgba::rgb(100.0, 80.0, 60.0))]))
            .add_systems(Update, (apply_lighting, apply_materials));
        let sun = app.world_mut().spawn((WeatherSun, DirectionalLight::default())).id();
        let window = app.world_mut().spawn((WindowSkylight, SpotLight::default())).id();
        let road = app.world_mut().spawn((NightSpot(1200.0), SpotLight::default(), Visibility::Hidden)).id();
        let camera = app.world_mut().spawn((Exposure::default(), DistanceFog::default(),
            EnvironmentMapLight { diffuse_map: default(), specular_map: default(), intensity: 0.0, rotation: Quat::IDENTITY })).id();
        app.world_mut().spawn((BakedSkyStrength(0.5), MeshMaterial3d(handle.clone())));
        for (id, preset) in [(1, CondoWeather::Night), (2, CondoWeather::Clear)] {
            {
                let mut state = app.world_mut().resource_mut::<AppliedWeather>();
                state.request(WeatherRequest::new(id, preset, 2.0).unwrap());
                state.step(2.0);
            }
            app.update();
            let world = app.world();
            let light = preset.lighting();
            let night = preset.sky().z;
            assert_eq!(world.get::<DirectionalLight>(sun).unwrap().illuminance, light.sun);
            assert_eq!(world.get::<SpotLight>(window).unwrap().intensity, light.window_lumens);
            assert_eq!(world.get::<SpotLight>(road).unwrap().intensity, 1200.0 * night);
            assert_eq!(world.get::<Visibility>(road).unwrap() == &Visibility::Hidden, night == 0.0);
            assert_eq!(world.get::<Exposure>(camera).unwrap().ev100, light.exposure);
            assert_eq!(world.get::<EnvironmentMapLight>(camera).unwrap().intensity, light.environment);
            assert!(matches!(world.get::<DistanceFog>(camera).unwrap().falloff,
                FogFalloff::Linear { start, end } if start == light.fog_start && end == light.fog_end));
            let material = world.resource::<Assets<StandardMaterial>>().get(&handle).unwrap();
            assert_eq!(material.lightmap_exposure, light.environment * 0.5);
            assert_eq!(material.emissive.red, night * 100.0);
        }
    }
}
