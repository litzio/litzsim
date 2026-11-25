#pragma once
#include <QJsonObject>
#include <QMap>
#include <QStringList>

// Field names carry SI units. Terminal order defines reference polarity.
struct Part {
    QStringList pins;
    QMap<QString, double> defaults;
};
const QMap<QString, Part>& library();

class Circuit {
public:
    QJsonObject document;
    static Circuit load(const QString& path);
    void save(const QString& path) const;
    void validate() const;
    QStringList missingPins() const;
    void move(const QString& id, int gridX, int gridY);
};
