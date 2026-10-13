package collector

import (
	"encoding/json"
	"fmt"
	"net/http"
	"sync/atomic"
	"time"

	"github.com/Skufik2007/methods-programming-lab14/go/internal/sinks"
)

type metrics struct {
	polls, pollErrors, readings, errors atomic.Int64
	flushes, flushErrors, batchRecords  atomic.Int64
	checkpointSaves                     atomic.Int64
}

// Handler — /metrics, /status, /healthz.
func (c *Collector) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, map[string]string{"status": "ok"})
	})
	mux.HandleFunc("GET /metrics", c.serveMetrics)
	mux.HandleFunc("GET /status", c.serveStatus)
	return mux
}

func (c *Collector) sinkStats() map[string]sinks.Stats {
	out := map[string]sinks.Stats{}
	for _, s := range c.sinks {
		if r, ok := s.Sink.(sinks.StatsReporter); ok {
			out[s.Name] = r.Stats()
		}
	}
	return out
}

func (c *Collector) assigned() []string {
	if p := c.devices.Load(); p != nil {
		return *p
	}
	return nil
}

func (c *Collector) serveMetrics(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "text/plain; version=0.0.4")
	ws := c.agg.Stats()
	p := func(typ, name string, v any) { fmt.Fprintf(w, "# TYPE %s %s\n%s %v\n", name, typ, name, v) }
	p("counter", "collector_polls_total", c.m.polls.Load())
	p("counter", "collector_poll_errors_total", c.m.pollErrors.Load())
	p("counter", "collector_readings_total", c.m.readings.Load())
	p("counter", "collector_errors_total", c.m.errors.Load())
	p("counter", "collector_flushes_total", c.m.flushes.Load())
	p("counter", "collector_flush_errors_total", c.m.flushErrors.Load())
	p("counter", "collector_flushed_records_total", c.m.batchRecords.Load())
	p("counter", "collector_checkpoint_saves_total", c.m.checkpointSaves.Load())
	p("counter", "collector_windows_closed_total", ws.Closed)
	p("counter", "collector_windows_closed_early_total", ws.EarlyClosed)
	p("counter", "collector_late_dropped_total", ws.LateDropped)
	p("counter", "collector_duplicates_skipped_total", ws.DupSkipped)
	p("counter", "collector_future_ts_total", ws.FutureClamped)
	p("gauge", "collector_windows_open", ws.Open)
	p("gauge", "collector_queue_length", len(c.in))
	p("gauge", "collector_queue_capacity", cap(c.in))
	p("gauge", "collector_devices_assigned", len(c.assigned()))
	fmt.Fprintf(w, "# TYPE collector_sink_bytes_total counter\n")
	for name, st := range c.sinkStats() {
		fmt.Fprintf(w, "collector_sink_bytes_total{sink=%q,kind=\"raw\"} %d\n", name, st.RawBytes)
		fmt.Fprintf(w, "collector_sink_bytes_total{sink=%q,kind=\"agg\"} %d\n", name, st.AggBytes)
	}
	fmt.Fprintf(w, "# TYPE collector_sink_records_total counter\n")
	for name, st := range c.sinkStats() {
		fmt.Fprintf(w, "collector_sink_records_total{sink=%q,kind=\"raw\"} %d\n", name, st.RawRecords)
		fmt.Fprintf(w, "collector_sink_records_total{sink=%q,kind=\"agg\"} %d\n", name, st.AggRecords)
	}
}

func (c *Collector) serveStatus(w http.ResponseWriter, _ *http.Request) {
	devs := c.assigned()
	shown := devs
	if len(shown) > 1000 {
		shown = shown[:1000]
	}
	writeJSON(w, map[string]any{
		"collector_id":     c.cfg.CollectorID,
		"started_at":       c.started.Format(time.RFC3339),
		"uptime_s":         c.clk.Now().Sub(c.started).Seconds(),
		"emit":             c.cfg.Emit,
		"polls":            c.m.polls.Load(),
		"poll_errors":      c.m.pollErrors.Load(),
		"readings":         c.m.readings.Load(),
		"errors":           c.m.errors.Load(),
		"flushes":          c.m.flushes.Load(),
		"windows":          c.agg.Stats(),
		"queue_length":     len(c.in),
		"queue_capacity":   cap(c.in),
		"sinks":            c.sinkStats(),
		"devices_assigned": len(devs),
		"devices":          shown,
	})
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}
