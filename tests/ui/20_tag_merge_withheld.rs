//! Misuse 20: merging policy tags. Expensify honours `action: "merge"` by
//! deleting every unlisted tag and answering 200, so there is no constructor
//! that promises a merge — `replace_all_*` is the whole vocabulary.

use expensify::{PolicyTag, TagLevel, TagsUpdate};

fn main() {
    let _ = TagsUpdate::merge_inline([TagLevel::new([PolicyTag::new("Gamma")])]);
}
