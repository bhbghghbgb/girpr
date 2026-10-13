//! The response types, including the fields a minimal client throws away.
//!
//! **M2.** `stats`, `compression`, `encryption`, `password` and `categories[]`
//! are modelled even though nothing here acts on all of them: a capture that
//! drops them cannot be used to answer the question they were kept for, which is
//! what the captured API was doing at that moment.