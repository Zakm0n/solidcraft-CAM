//! A CAM coordinate system's visibility (`visibility.hidden_coordinate_systems`): the ids of the
//! coordinate systems whose triad the view does not draw, so the browser can show and hide one
//! (`browser.visibility`) as it does a joint's glyph.
//! Older designs need no transformation: a design without the key has every coordinate system
//! drawn, and keeps saving without it.
