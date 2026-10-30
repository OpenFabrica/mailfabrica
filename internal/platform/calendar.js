
ObjC.import('EventKit');
ObjC.import('AppKit');
function str(v) { try { const j = v.js; return typeof j === 'string' ? j : ''; } catch (e) { return ''; } }
function hex(c) {
  try {
    const s = c.colorUsingColorSpace($.NSColorSpace.sRGBColorSpace);
    const h = v => ('0' + Math.round(Math.max(0, Math.min(1, v)) * 255).toString(16)).slice(-2);
    return '#' + h(s.redComponent) + h(s.greenComponent) + h(s.blueComponent);
  } catch (e) { return ''; }
}
function run(argv) {
  let st = Number($.EKEventStore.authorizationStatusForEntityType(0));
  if (st === 0) {
    const s0 = $.EKEventStore.alloc.init;
    let done = false;
    const cb = function (granted, err) { done = true; };
    if (s0.respondsToSelector('requestFullAccessToEventsWithCompletion:')) s0.requestFullAccessToEventsWithCompletion(cb);
    else s0.requestAccessToEntityTypeCompletion(0, cb);
    const until = $.NSDate.dateWithTimeIntervalSinceNow(120);
    while (!done && $.NSDate.date.compare(until) < 0) $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.1));
    st = Number($.EKEventStore.authorizationStatusForEntityType(0));
  }
  if (st !== 3) return JSON.stringify({denied: true, status: st});
  const store = $.EKEventStore.alloc.init;
  const cals = store.calendarsForEntityType(0);
  const calendars = [];
  for (let i = 0; i < cals.count; i++) {
    const c = cals.objectAtIndex(i);
    calendars.push({id: str(c.calendarIdentifier), title: str(c.title), source: str(c.source.title), color: hex(c.color)});
  }
  const pred = store.predicateForEventsWithStartDateEndDateCalendars(
    $.NSDate.dateWithTimeIntervalSince1970(Number(argv[0])), $.NSDate.dateWithTimeIntervalSince1970(Number(argv[1])), $());
  const evs = store.eventsMatchingPredicate(pred);
  const events = [];
  for (let i = 0; i < evs.count && i < 5000; i++) {
    const v = evs.objectAtIndex(i);
    events.push({id: str(v.eventIdentifier), calendar: str(v.calendar.calendarIdentifier), title: str(v.title),
      start: Math.floor(v.startDate.timeIntervalSince1970), end: Math.floor(v.endDate.timeIntervalSince1970),
      allDay: v.allDay === true, location: str(v.location)});
  }
  return JSON.stringify({calendars: calendars, events: events});
}
