#pragma once
#include "circuit.h"
#include <functional>
using Sample = std::function<void(double, const QMap<QString,double>&)>;
void simulate(const Circuit& circuit, const Sample& sample);
void simulateCsv(const Circuit& circuit, const QString& filename);
