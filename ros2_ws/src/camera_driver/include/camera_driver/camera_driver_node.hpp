#pragma once

#include <chrono>
#include <memory>
#include <string>

#include <opencv2/videoio.hpp>
#include <rclcpp/rclcpp.hpp>
#include <sensor_msgs/msg/image.hpp>

namespace camera_driver
{

class CameraDriverNode : public rclcpp::Node
{
public:
    explicit CameraDriverNode(const rclcpp::NodeOptions & options = rclcpp::NodeOptions());

private:
    void captureLoop();
    bool openCamera();

    cv::VideoCapture capture_;
    rclcpp::Publisher<sensor_msgs::msg::Image>::SharedPtr publisher_;
    rclcpp::TimerBase::SharedPtr timer_;

    int device_id_;
    int fps_;
    int width_;
    int height_;
    std::string topic_;
    std::string frame_id_;
};

}  // namespace camera_driver