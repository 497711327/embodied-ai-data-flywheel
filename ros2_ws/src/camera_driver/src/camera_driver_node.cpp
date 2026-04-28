#include "camera_driver/camera_driver_node.hpp"

#include <algorithm>
#include <chrono>
#include <memory>
#include <string>

#include <cv_bridge/cv_bridge.h>
#include <opencv2/imgproc.hpp>
#include <opencv2/videoio.hpp>
#include <rclcpp/rclcpp.hpp>
#include <sensor_msgs/image_encodings.hpp>

namespace camera_driver
{

CameraDriverNode::CameraDriverNode(const rclcpp::NodeOptions & options)
: Node("camera_driver_node", options),
	device_id_(this->declare_parameter<int>("device_id", 0)),
	fps_(this->declare_parameter<int>("fps", 30)),
	width_(this->declare_parameter<int>("width", 640)),
	height_(this->declare_parameter<int>("height", 480)),
	topic_(this->declare_parameter<std::string>("topic", "/camera/front/image_raw")),
	frame_id_(this->declare_parameter<std::string>("frame_id", "camera_front"))
{
	fps_ = std::max(1, fps_);
	publisher_ = this->create_publisher<sensor_msgs::msg::Image>(topic_, rclcpp::SensorDataQoS());

	if (!openCamera()) {
		RCLCPP_ERROR(
			this->get_logger(),
			"Failed to open camera device %d. In WSL, check that /dev/video%d exists and is readable.",
			device_id_, device_id_);
	}

	const auto period = std::chrono::milliseconds(1000 / fps_);
	timer_ = this->create_wall_timer(period, std::bind(&CameraDriverNode::captureLoop, this));

	RCLCPP_INFO(
		this->get_logger(), "Publishing camera %d to %s at %d FPS (%dx%d), frame_id=%s",
		device_id_, topic_.c_str(), fps_, width_, height_, frame_id_.c_str());
}

bool CameraDriverNode::openCamera()
{
	if (capture_.isOpened()) {
		return true;
	}

	if (!capture_.open(device_id_, cv::CAP_V4L2)) {
		return false;
	}

	capture_.set(cv::CAP_PROP_FRAME_WIDTH, width_);
	capture_.set(cv::CAP_PROP_FRAME_HEIGHT, height_);
	capture_.set(cv::CAP_PROP_FPS, fps_);
	return capture_.isOpened();
}

void CameraDriverNode::captureLoop()
{
	if (!capture_.isOpened() && !openCamera()) {
		RCLCPP_WARN_THROTTLE(
			this->get_logger(), *this->get_clock(), 5000,
			"Camera device %d is not available yet", device_id_);
		return;
	}

	cv::Mat frame;
	if (!capture_.read(frame) || frame.empty()) {
		RCLCPP_WARN_THROTTLE(
			this->get_logger(), *this->get_clock(), 5000,
			"Camera device %d returned an empty frame", device_id_);
		return;
	}

	cv_bridge::CvImage image;
	image.header.stamp = this->now();
	image.header.frame_id = frame_id_;
	image.encoding = sensor_msgs::image_encodings::BGR8;
	image.image = frame;

	publisher_->publish(*image.toImageMsg());
}

}  // namespace camera_driver

int main(int argc, char ** argv)
{
	rclcpp::init(argc, argv);
	rclcpp::spin(std::make_shared<camera_driver::CameraDriverNode>());
	rclcpp::shutdown();
	return 0;
}
