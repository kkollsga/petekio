//! `validate` — bounds / validity-range checks on normalized inputs, and the
//! provenance/hardness flagging that lets the consumer mark hard data vs
//! interpolated vs defaulted. The second half of the path to
//! [`ModelInputs`](super::model_inputs::ModelInputs).
//!
//! GATE-0: the layer is declared here; the range tables and the flagging pass
//! land per `dev-docs`. (Tracked from the consumer side as the petekio
//! "validate" gap.)
