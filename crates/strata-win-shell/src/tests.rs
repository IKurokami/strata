use super::*;

#[test]
fn recycle_sink_vetoes_destructive_shell_fallback_before_deletion() {
    let outcomes = Rc::new(RefCell::new(Outcomes::default()));
    let sink: IFileOperationProgressSink = Sink {
        outcomes: outcomes.clone(),
        refuse_existing_destination: false,
        expect_recycled_item: true,
    }
    .into();
    // SAFETY: The sink's preflight callback only inspects flags and never dereferences this absent item.
    assert!(unsafe { sink.PreDeleteItem(0, None) }.is_err());
    assert_eq!(outcomes.borrow().failure, Some(E_ABORT));
    // SAFETY: The same flag-only callback accepts explicit Shell recycling.
    assert!(unsafe { sink.PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, None) }.is_ok());
    let permanent: IFileOperationProgressSink = Sink {
        outcomes: Rc::new(RefCell::new(Outcomes::default())),
        refuse_existing_destination: false,
        expect_recycled_item: false,
    }
    .into();
    // SAFETY: Explicit permanent deletion uses the same callback without requiring recycle flags.
    assert!(unsafe { permanent.PreDeleteItem(0, None) }.is_ok());
}
