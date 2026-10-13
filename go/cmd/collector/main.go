// Команда collector — сборщик показаний (docs/ARCHITECTURE.md, раздел 4).
package main

import (
	"context"
	"flag"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	"github.com/Skufik2007/methods-programming-lab14/go/internal/clock"
	"github.com/Skufik2007/methods-programming-lab14/go/internal/collector"
	"github.com/Skufik2007/methods-programming-lab14/go/internal/config"
	"github.com/Skufik2007/methods-programming-lab14/go/internal/sinks"
	"github.com/Skufik2007/methods-programming-lab14/go/internal/sinks/files"
)

func main() { os.Exit(run()) }

func run() int {
	duration := flag.Duration("duration", 0, "завершиться через указанное время тем же путём, что и по сигналу (0 — работать до сигнала)")
	flag.Parse()

	log := slog.New(slog.NewJSONHandler(os.Stderr, nil))
	cfg, err := config.LoadCollector(os.Getenv)
	if err != nil {
		fmt.Fprintln(os.Stderr, "ошибка конфигурации:\n"+err.Error())
		return 2
	}
	log = log.With("component", "collector", "collector_id", cfg.CollectorID)
	clk := clock.Real{}

	// Новые стоки (nats, flight) регистрируются здесь одной строкой.
	reg := sinks.NewRegistry()
	reg.Register("files", files.Factory)
	named, err := reg.Build(cfg.Sinks, sinks.Env{Config: cfg, Clock: clk, Log: log})
	if err != nil {
		fmt.Fprintln(os.Stderr, "ошибка конфигурации: "+err.Error())
		return 2
	}
	cps, err := collector.NewFileStore(filepath.Join(cfg.OutDir, "checkpoints.json"))
	if err != nil {
		fmt.Fprintln(os.Stderr, "чекпоинты: "+err.Error())
		return 1
	}
	src := collector.NewHTTPSource(cfg.SimURL, cfg.HTTPTimeout, cfg.Workers)
	c, err := collector.New(collector.Deps{
		Config: cfg, Log: log, Clock: clk, Source: src, Checkpoints: cps, Sinks: named,
		Assigner: collector.NewStaticAssigner(cfg.DeviceList, src.Devices),
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 1
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if *duration > 0 {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(ctx, *duration)
		defer cancel()
	}
	start := time.Now()
	if err := c.Run(ctx); err != nil {
		log.Error("завершение с ошибкой", "err", err, "uptime", time.Since(start).String())
		return 1
	}
	return 0
}
