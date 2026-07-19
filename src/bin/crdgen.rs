//! Generates the CustomResourceDefinition YAML for the `FoldingAtHome` resource.
//!
//! Run with `cargo run --bin crdgen > deploy/crd.yaml`.

use fah_operator::crd::FoldingAtHome;
use kube::CustomResourceExt;

fn main() {
    let crd = FoldingAtHome::crd();
    print!(
        "{}",
        serde_yaml::to_string(&crd).expect("serialize CRD to YAML")
    );
}
