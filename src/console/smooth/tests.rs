use super::*;

fn shown_after(ease: &mut ProgressEase, target: &Status, ticks: usize) -> Vec<u8> {
    (0..ticks)
        .map(|_| {
            ease.tick(target);
            percent_of(&ease.displayed(target)).unwrap()
        })
        .collect()
}

#[test]
fn the_first_report_is_shown_as_is() {
    let ease = ProgressEase::default();

    assert_eq!(
        ease.displayed(&Status::Compiling(40)),
        Status::Compiling(40)
    );
}

#[test]
fn a_jump_is_spread_over_several_ticks() {
    let mut ease = ProgressEase::default();
    ease.tick(&Status::Compiling(0));

    let target = Status::Compiling(60);
    assert_eq!(ease.displayed(&target), Status::Compiling(0));

    let steps = shown_after(&mut ease, &target, 70);
    assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
    assert_eq!(steps[0], 1, "{steps:?}");
    assert_eq!(*steps.last().unwrap(), 60);
}

#[test]
fn every_number_between_the_old_and_new_value_is_shown() {
    let mut ease = ProgressEase::default();
    ease.tick(&Status::Compiling(25));

    let target = Status::Compiling(35);
    let steps = shown_after(&mut ease, &target, 12);

    assert_eq!(&steps[..10], &[26, 27, 28, 29, 30, 31, 32, 33, 34, 35]);
    assert_eq!(&steps[10..], &[35, 35]);
}

#[test]
fn each_step_is_one_point_and_stops_at_the_target() {
    assert_eq!(next_step(0, 60), 1);
    assert_eq!(next_step(57, 60), 58);
    assert_eq!(next_step(59, 60), 60);
    assert_eq!(next_step(60, 60), 60);
}

#[test]
fn the_shown_value_never_passes_the_reported_one() {
    let mut ease = ProgressEase::default();
    ease.tick(&Status::Building(10));

    let steps = shown_after(&mut ease, &Status::Building(12), 10);

    assert!(steps.iter().all(|p| *p <= 12), "{steps:?}");
}

#[test]
fn a_new_phase_starts_at_its_reported_value() {
    let mut ease = ProgressEase::default();
    shown_after(&mut ease, &Status::Compiling(90), 1);

    assert_eq!(ease.displayed(&Status::Building(0)), Status::Building(0));
}

#[test]
fn a_report_that_moves_backwards_snaps_to_it() {
    let mut ease = ProgressEase::default();
    ease.tick(&Status::Compiling(0));
    shown_after(&mut ease, &Status::Compiling(80), 20);

    assert_eq!(ease.displayed(&Status::Compiling(0)), Status::Compiling(0));
}

#[test]
fn statuses_without_a_percentage_pass_through_and_reset_the_ease() {
    let mut ease = ProgressEase::default();
    ease.tick(&Status::Compiling(0));
    ease.tick(&Status::Launching);

    assert_eq!(ease.displayed(&Status::Launching), Status::Launching);
    assert_eq!(
        ease.displayed(&Status::Compiling(50)),
        Status::Compiling(50)
    );
}
