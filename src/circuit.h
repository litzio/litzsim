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
    QString insert(const QString& type,int x,int y);
    void rotate(const QString& id);
    void connectPins(const QString& a,const QString& b);
    void disconnectPin(const QString& pin);
    void erase(const QString& id);
};
