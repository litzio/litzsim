#pragma once
#include <QFile>
#include <QSaveFile>
#include <QPointF>
#include <QStringList>
#include <map>
#include <vector>

// Version one stores little-endian IEEE doubles in row order on Windows.
class WaveWriter {
public:
    WaveWriter(const QString& file,const QStringList& channels);
    void append(const std::vector<double>& row);
    void finish();
private:
    QSaveFile file;
    int columns;
    qint64 count=0;
    double lastTime=0;
    std::vector<double> minimum,maximum;
    std::vector<quint16> offsets,index;
};
class Waves {
public:
    explicit Waves(const QString& path);
    QStringList channels;
    qint64 rows=0;
    double at(qint64 row,int col) const;
    std::vector<QPointF> envelope(double from,double to,int y,int pixels) const;
    std::vector<qint64> envelopeRows(double from,double to,int y,int pixels) const;
private:
    QFile file;
    const double* data=nullptr;
    const quint16* index=nullptr;
    struct Extremes {qint64 low,high;double minimum,maximum;};
    mutable std::map<int,std::vector<Extremes>> summaries;
};
void importCsv(const QString& input,const QString& output);
