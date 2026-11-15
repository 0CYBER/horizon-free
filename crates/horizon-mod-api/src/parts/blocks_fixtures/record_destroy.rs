fn record_destroy(event: &OnDestroyedByPlayerEvent<'_>) {
    let tool = event.tool.map_or(0, |tool| tool.count());
    DESTROYS.with(|r| r.borrow_mut().push((event.will_harvest, tool)));
}
