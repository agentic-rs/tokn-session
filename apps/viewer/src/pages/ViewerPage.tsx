import { Conversation } from "../components/Conversation";
import { Inspector } from "../components/Inspector";
import { Sidebar } from "../components/Sidebar";
import { useViewerState } from "../lib/useViewerState";

export function ViewerPage() {
  const viewer = useViewerState();

  return (
    <div className="viewer-shell" data-inspector-open={viewer.inspectorOpen}>
      <div className="sidebar-shell" data-mobile-open={viewer.mobileSidebarOpen}>
        <Sidebar
          enabled_providers={viewer.enabledProviders}
          error={viewer.sessionsError}
          has_more={viewer.sessionsCursor !== null}
          is_loading={viewer.sessionsLoading}
          is_loading_more={viewer.sessionsLoadingMore}
          on_children_load={viewer.loadSessionChildren}
          on_children_load_more={viewer.loadMoreSessionChildren}
          on_children_retry={viewer.retrySessionChildren}
          on_load_more={viewer.loadMoreSessions}
          on_provider_toggle={viewer.toggleProvider}
          on_retry={viewer.retrySessions}
          on_search_change={viewer.setSearch}
          on_session_select={viewer.selectSession}
          search={viewer.search}
          session_children={viewer.sessionChildren}
          selected_session_key={viewer.selectedSessionKey}
          sessions={viewer.sessions}
          source_errors={viewer.sourceErrors}
        />
      </div>

      {viewer.mobileSidebarOpen ? (
        <button
          aria-label="Close sessions"
          className="sidebar-backdrop"
          onClick={() => viewer.setMobileSidebarOpen(false)}
          type="button"
        />
      ) : null}

      <Conversation
        error={viewer.eventsError}
        events={viewer.events}
        expanded_detail={viewer.expandedDetail}
        expanded_detail_error={viewer.expandedDetailError}
        expanded_detail_loading={viewer.expandedDetailLoading}
        expanded_event_key={viewer.expandedEventKey}
        has_newer={viewer.newerCursor !== null}
        has_older={viewer.olderCursor !== null}
        history_status={viewer.historyStatus}
        initial_page_loaded={viewer.initialPageLoaded}
        inspector_open={viewer.inspectorOpen}
        is_loading={viewer.eventsLoading}
        is_loading_newer={viewer.newerLoading}
        is_loading_older={viewer.olderLoading}
        on_event_select={viewer.selectEvent}
        on_event_toggle={viewer.toggleEventExpanded}
        on_open_subagent={viewer.openSubagent}
        on_inspector_toggle={viewer.toggleInspector}
        on_load_newer={viewer.loadNewerEvents}
        on_load_older={viewer.loadOlderEvents}
        on_retry={viewer.retryEvents}
        on_retry_expanded_detail={viewer.retryExpandedDetail}
        on_sidebar_open={() => viewer.setMobileSidebarOpen(true)}
        on_trajectory_load_newer={viewer.loadNewerTrajectoryEvents}
        on_trajectory_load_older={viewer.loadOlderTrajectoryEvents}
        on_trajectory_retry={viewer.retryTrajectoryEvents}
        on_trajectory_event_toggle={viewer.toggleTrajectoryEventExpanded}
        on_trajectory_retry_expanded_detail={viewer.retryExpandedTrajectoryDetail}
        selected_event_key={viewer.selectedEventKey}
        session={viewer.selectedSession}
        total_events={viewer.totalEvents}
        trajectory_expanded_detail={viewer.expandedTrajectoryDetail}
        trajectory_expanded_detail_error={viewer.expandedTrajectoryDetailError}
        trajectory_expanded_detail_loading={viewer.expandedTrajectoryDetailLoading}
        trajectory_expanded_event_key={viewer.expandedTrajectoryEventKey}
        trajectory_expanded_key={viewer.expandedTrajectoryKey}
        trajectory_pages={viewer.trajectoryPages}
      />

      <Inspector
        detail={viewer.detail}
        error={viewer.detailError}
        event={viewer.selectedEvent}
        is_loading={viewer.detailLoading}
        is_open={viewer.inspectorOpen}
        on_close={viewer.closeInspector}
        on_retry={viewer.retryDetail}
      />
    </div>
  );
}
