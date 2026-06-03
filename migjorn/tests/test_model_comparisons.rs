use migjorn::Model;

#[test]
fn test_model_semantic_equality_ignores_formatting_and_comments() {
    let text_a = "Title A\n\
1 1 -1 -10 IMP:N=1 U=2\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1\n\
TR7 1 0 0\n";

    let text_b = "Totally different title\n\
C card-level comments and whitespace changes should not affect equality\n\
  1   1   -1.0   -10   IMP:N = 1   U = 2   $ trailing comment\n\
\n\
C surface comment\n\
10   PX   0.0\n\
\n\
c data comment\n\
M1   1001.80c   1.0\n\
TR7   1.0   0.0   0.0\n";

    let model_a = Model::from_text("first_path.mcnp", text_a).unwrap();
    let model_b = Model::from_text("different_path.mcnp", text_b).unwrap();

    assert!(model_a == model_b);
}

#[test]
fn test_model_semantic_equality_detects_real_content_changes() {
    let base = "Title\n\
1 1 -1 -10 IMP:N=1 U=2\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1\n\
TR7 1 0 0\n";

    let changed = "Title\n\
1 1 -1 -10 IMP:N=1 U=2\n\
\n\
10 PX 0\n\
\n\
M1 1001.80c 1\n\
TR8 1 0 0\n";

    let model_base = Model::from_text("base.mcnp", base).unwrap();
    let model_changed = Model::from_text("changed.mcnp", changed).unwrap();

    assert!(model_base != model_changed);
}
