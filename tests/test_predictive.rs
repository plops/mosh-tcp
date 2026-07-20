use mosh_tcp::predictive::LocalPredictor;
use std::io;

#[test]
fn test_alternate_screen_detection_and_suspension() {
    let mut predictor = LocalPredictor::new(true);
    let mut stdout = io::stdout();

    assert!(!predictor.is_suspended());

    // Simulate server frame enabling alternate screen (\x1b[?1049h - e.g. Emacs, Browsh, Vim)
    let frame_alt_on = b"\x1b[?1049h\x1b[H\x1b[2JWelcome to Emacs";
    predictor.inspect_server_frame(frame_alt_on, &mut stdout);

    assert!(predictor.is_suspended());

    // Verify that keystrokes while suspended do not increment active predictions
    predictor.handle_keystroke(b"iHello World\x1b").unwrap();
    assert_eq!(predictor.active_predictions(), 0);

    // Simulate server frame disabling alternate screen (\x1b[?1049l - exiting editor)
    let frame_alt_off = b"\x1b[?1049l\x1b[H\x1b[2Jshell$ ";
    predictor.inspect_server_frame(frame_alt_off, &mut stdout);

    assert!(!predictor.is_suspended());

    // Keystrokes in normal shell should predict again
    predictor.handle_keystroke(b"ls").unwrap();
    assert_eq!(predictor.active_predictions(), 2);
}

#[test]
fn test_control_character_clears_predictions() {
    let mut predictor = LocalPredictor::new(true);

    // Keystrokes accumulate predictions
    predictor.handle_keystroke(b"cat").unwrap();
    assert_eq!(predictor.active_predictions(), 3);

    // Sending Arrow key (ESC [ A) clears predictions before cursor moves
    predictor.handle_keystroke(&[27, 91, 65]).unwrap();
    assert_eq!(predictor.active_predictions(), 0);
}
