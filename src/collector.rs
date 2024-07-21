use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, RwLock},
};

use metrics::{Counter, Gauge, Histogram, Key, KeyName, Metadata, Recorder, SharedString, Unit};
use prometheus_client::{
    collector::Collector,
    encoding::{DescriptorEncoder, EncodeMetric},
    registry::Unit as PrometheusUnit,
};

use crate::{metrics::*, utils::*};

pub type Map<K, V> = Arc<RwLock<HashMap<K, V>>>;
pub type Label = (String, String);

#[derive(Debug, Default)]
struct Descriptor {
    help: String,
    unit: Option<PrometheusUnit>,
}

impl Descriptor {
    fn new(help: String, unit: Option<Unit>) -> Self {
        Self {
            help,
            unit: unit.map(convert_unit_to_prometheus),
        }
    }
}

/// This module provides compatibility with the `metrics` crate.
/// It registers as a `Collector` with the `prometheus_client` crate
/// and implements the `Recorder` trait of the `metrics` crate.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    metrics: Map<KeyName, Vec<(Vec<Label>, Metric)>>,
    descriptors: Map<KeyName, Descriptor>,
}

impl Clone for MetricsCollector {
    fn clone(&self) -> Self {
        Self {
            metrics: Arc::clone(&self.metrics),
            descriptors: Arc::clone(&self.descriptors),
        }
    }
}

impl Collector for MetricsCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), fmt::Error> {
        for (key_name, metrics) in self.metrics.read().unwrap().iter() {
            // Find descriptor for the metric.
            let descriptors = self.descriptors.read().unwrap();
            let (help, unit) = descriptors
                .get(key_name)
                .map(|d| (d.help.as_str(), d.unit.as_ref()))
                .unwrap_or_else(|| ("", None));

            // Gather statistics about the metrics.
            if metrics.is_empty() {
                continue;
            }
            let metric_type = metrics[0].1.metric_type();
            // If there is more than one entry, this is always true.
            let has_labels = !metrics[0].0.is_empty();

            // Encode descriptor and metric.
            let mut descriptor_encoder =
                encoder.encode_descriptor(key_name.as_str(), help, unit, metric_type)?;

            // Encode metrics for this key.
            // If labels are present, encode the metric as a family.
            if has_labels {
                for (labels, metric) in metrics {
                    let metric_encoder = descriptor_encoder.encode_family(labels)?;
                    metric.encode(metric_encoder)?;
                }
            } else {
                let metric = &metrics[0].1;
                metric.encode(descriptor_encoder)?;
            }
        }
        Ok(())
    }
}

impl MetricsCollector {
    fn register(&self, key: &Key, metric: Metric) {
        let (key_name, labels) = key.clone().into_parts();
        let labels = convert_labels_to_prometheus(labels);

        let mut metrics = self.metrics.write().unwrap();
        let entry = metrics.entry(key_name).or_default();

        // Make sure that all metrics for a key have the same type
        // and that labels are set on duplicate entries..
        assert!(
            entry.is_empty()
                || (entry[0].1.metric_type().as_str() == metric.metric_type().as_str()
                    && !entry[0].0.is_empty()
                    && !labels.is_empty()),
            "Registering a metric with a different type or missing labels: `{:?}`",
            key
        );
        entry.push((labels, metric));
    }

    fn describe(&self, key: &KeyName, unit: Option<Unit>, description: SharedString) {
        assert!(
            self.descriptors
                .write()
                .unwrap()
                .insert(key.clone(), Descriptor::new(description.into_owned(), unit))
                .is_none(),
            "Registering a duplicate metric descriptor: `{:?}`",
            key
        );
    }
}

impl Recorder for MetricsCollector {
    fn describe_counter(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.describe(&key, unit, description)
    }

    fn describe_gauge(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.describe(&key, unit, description)
    }

    fn describe_histogram(&self, key: KeyName, unit: Option<Unit>, description: SharedString) {
        self.describe(&key, unit, description)
    }

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        let counter = Arc::new(MetricsCounter::default());
        self.register(key, Metric::Counter(Arc::clone(&counter)));
        Counter::from_arc(counter)
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
        let gauge = Arc::new(MetricsGauge::default());
        self.register(key, Metric::Gauge(Arc::clone(&gauge)));
        Gauge::from_arc(gauge)
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
        let hist = Arc::new(MetricsHistogram::default());
        self.register(key, Metric::Histogram(Arc::clone(&hist)));
        Histogram::from_arc(hist)
    }
}
